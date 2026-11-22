//! Key action handlers: list/page/queue navigation and playback.

use super::super::*;

/// Description reader steps (legacy QML `DescriptionOverlay` grammar): `j`/`k`
/// move one text line, the page motions move 90% of the viewport.
const DESCRIPTION_LINE_STEP: f32 = 22.0;
const DESCRIPTION_PAGE_RATIO: f32 = 0.9;

/// Rows one PageUp/PageDown motion crosses on the list surfaces.
const LIST_PAGE_ROWS: usize = 10;

/// The next description reader offset, clamped into `0..=max`.
pub(in crate::ui) fn description_offset(current: f32, delta: f32, max: f32) -> f32 {
    (current + delta).clamp(0.0, max.max(0.0))
}

fn contrast_after_step(current: f64, direction: i8) -> f64 {
    (((current + f64::from(direction) * 0.1) * 10.0).round() / 10.0).clamp(-1.0, 1.0)
}

fn signed_percent(value: f64) -> String {
    if value.abs() < 0.005 {
        "0%".to_string()
    } else {
        format!(
            "{}{:.0}%",
            if value > 0.0 { "+" } else { "" },
            value * 100.0
        )
    }
}

#[derive(Clone, Copy)]
enum SettingsRowAction {
    Shuffle,
    Repeat,
    Crossfade,
    Length,
    Curve,
    AlbumOnly,
    Normalize,
    Eq,
    ColorContrast,
    ColorSaturation,
    Scrobble,
    BitPerfect,
    Exclusive,
    Cache,
    Quality,
    FixedRate,
    Format,
    Sleep30,
    SleepOff,
    Close,
}

impl AppRoot {
    pub(in crate::ui) fn on_quit(
        &mut self,
        _: &Quit,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.remove_window();
    }

    pub(in crate::ui) fn on_move_down(
        &mut self,
        _: &MoveDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_open {
            let rows = self.settings_rows().len();
            self.settings_cursor = (self.settings_cursor + 1).min(rows.saturating_sub(1));
            cx.notify();
            return;
        }
        self.status_focus = None;
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(1);
            cx.notify();
            return;
        }
        if self.description.is_some() {
            self.scroll_description(DESCRIPTION_LINE_STEP);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_list.cursor = self
                .queue_list
                .cursor
                .saturating_add(1)
                .min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() {
            self.scroll_lyrics_by(self.lyrics_line_step(), cx);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            if !self.visual_mode {
                page.list.selection.clear();
            }
            page.select(page.list.cursor.saturating_add(1));
            self.hovered_page = None;
            self.follow_page(true);
        } else {
            self.select(self.view.list.cursor.saturating_add(1));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_move_up(
        &mut self,
        _: &MoveUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_open {
            self.settings_cursor = self.settings_cursor.saturating_sub(1);
            cx.notify();
            return;
        }
        self.status_focus = None;
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(-1);
            cx.notify();
            return;
        }
        if self.description.is_some() {
            self.scroll_description(-DESCRIPTION_LINE_STEP);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_list.cursor = self.queue_list.cursor.saturating_sub(1);
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() {
            self.scroll_lyrics_by(-self.lyrics_line_step(), cx);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            if !self.visual_mode {
                page.list.selection.clear();
            }
            page.select(page.list.cursor.saturating_sub(1));
            self.hovered_page = None;
            self.follow_page(true);
        } else {
            self.select(self.view.list.cursor.saturating_sub(1));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_top(&mut self, _: &Top, _window: &mut Window, cx: &mut Context<Self>) {
        if self.description.is_some() {
            self.scroll_description(-f32::MAX);
        } else if self.lyrics_scroll_focused() && !self.queue_open {
            self.lyrics_top(cx);
        } else if self.queue_open {
            self.land_queue(0);
        } else if self.page.is_some() {
            self.land_page(0);
        } else {
            self.land_library(0);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_bottom(
        &mut self,
        _: &Bottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.description.is_some() {
            self.scroll_description(f32::MAX);
        } else if self.lyrics_scroll_focused() && !self.queue_open {
            self.lyrics_bottom(cx);
        } else if self.queue_open {
            let last = self.player.len().saturating_sub(1);
            self.land_queue(last);
        } else if let Some(page) = self.page.as_ref() {
            let last = page.rows.len().saturating_sub(1);
            self.land_page(last);
        } else {
            let last = self.view.rows.len().saturating_sub(1);
            self.land_library(last);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_scope_next(
        &mut self,
        _: &ScopeNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(self.view.scope.next());
        cx.notify();
    }

    pub(in crate::ui) fn on_scope_prev(
        &mut self,
        _: &ScopePrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(self.view.scope.prev());
        cx.notify();
    }

    pub(in crate::ui) fn on_order_next(
        &mut self,
        _: &OrderNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search.is_some() {
            self.move_search_scope(1);
            cx.notify();
            return;
        }
        self.cycle_order(true);
        cx.notify();
    }

    pub(in crate::ui) fn on_order_prev(
        &mut self,
        _: &OrderPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search.is_some() {
            self.move_search_scope(-1);
            cx.notify();
            return;
        }
        self.cycle_order(false);
        cx.notify();
    }

    pub(in crate::ui) fn on_refresh(
        &mut self,
        _: &Refresh,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_sync(SyncMode::Incremental);
        cx.notify();
    }

    pub(in crate::ui) fn cycle_fullscreen_focus(&mut self, backward: bool) {
        let mut order = vec![FullscreenFocus::Cover, FullscreenFocus::Title];
        if self.lyrics_inline() && !self.lyrics_overlay {
            order.push(FullscreenFocus::Lyrics);
        }
        let position = order
            .iter()
            .position(|focus| *focus == self.fullscreen_focus)
            .unwrap_or(0);
        let next = if backward {
            (position + order.len() - 1) % order.len()
        } else {
            (position + 1) % order.len()
        };
        self.fullscreen_focus = order[next];
    }

    pub(in crate::ui) fn on_toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            self.request_current_lyrics();
            self.ensure_marks();
            self.refresh_fullscreen_album_state();
        } else {
            self.release_lyrics_hold();
        }
        cx.notify();
    }

    /// Any navigation to another place retracts the fullscreen playback view
    /// (and releases its lyrics hold).
    pub(in crate::ui) fn retract_playback(&mut self) {
        if self.fullscreen {
            self.fullscreen = false;
            self.release_lyrics_hold();
        }
    }

    /// Optimistically apply the start of a download toggle so the row marks
    /// react at once: a start shows the loading glyph, a removal drops the
    /// downloaded glyph before the ledger read settles.
    pub(in crate::ui) fn apply_download_started(&mut self, kind: &str, id: &str, on: bool) {
        if id.is_empty() {
            return;
        }
        let row_kind = match kind {
            "song" => RowKind::Song,
            "album" => RowKind::Album,
            "artist" => RowKind::Artist,
            _ => return,
        };
        let mut songs = std::collections::HashSet::new();
        let mut albums = std::collections::HashSet::new();
        let mut row_ids = std::collections::HashSet::new();
        let mut patch = |row: &mut Row, container: bool| {
            let hit = container
                || row.id == id
                || row.song_id == id
                || (!row.album_id.is_empty() && row.album_id == id);
            if !hit {
                return;
            }
            if !row.id.is_empty() {
                row_ids.insert(row.id.clone());
            }
            if !row.song_id.is_empty() {
                songs.insert(row.song_id.clone());
            }
            if !row.album_id.is_empty() {
                albums.insert(row.album_id.clone());
            }
        };
        for row in &mut self.view.rows.live {
            patch(row, false);
        }
        if let Some(search) = self.search.as_mut() {
            for row in &mut search.rows {
                patch(row, false);
            }
        }
        if let Some(page) = self.page.as_mut() {
            let container = page.descriptor.id == id
                && matches!(
                    (page.descriptor.kind, row_kind),
                    (PageKind::Album, RowKind::Album) | (PageKind::Artist, RowKind::Artist)
                );
            if container {
                page.container_cached = on;
                page.container_downloading = true;
            }
            for row in &mut page.rows {
                patch(row, container);
            }
        }
        if on {
            self.marks.pending.insert(id.to_string());
            self.marks.pending.extend(songs);
            self.marks.pending.extend(albums);
            self.marks.pending.extend(row_ids);
            return;
        }
        self.marks.cleared.insert(id.to_string());
        self.marks.songs_done.remove(id);
        self.marks.albums_done.remove(id);
        self.marks.pending.remove(id);
        self.marks.cleared.extend(songs.iter().cloned());
        self.marks.cleared.extend(albums.iter().cloned());
        self.marks.cleared.extend(row_ids.iter().cloned());
        for song in &songs {
            self.marks.songs_done.remove(song);
            self.marks.pending.remove(song);
        }
        for album in &albums {
            self.marks.albums_done.remove(album);
            self.marks.pending.remove(album);
        }
        for row_id in &row_ids {
            self.marks.pending.remove(row_id);
        }
    }

    pub(in crate::ui) fn on_crossfade_toggle(
        &mut self,
        _: &CrossfadeToggle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.toggle_crossfade();
        cx.notify();
    }

    pub(in crate::ui) fn on_normalization_cycle(
        &mut self,
        _: &NormalizationCycle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.cycle_normalization();
        cx.notify();
    }

    pub(in crate::ui) fn on_settings_overlay(
        &mut self,
        _: &SettingsOverlay,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_open = !self.settings_open;
        self.settings_cursor = 0;
        cx.notify();
    }

    fn settings_actions(&self) -> Vec<(String, String, SettingsRowAction)> {
        let s = &self.settings;
        let yes_no = |value: bool| if value { "On" } else { "Off" }.to_string();
        let repeat = match self.player.repeat() {
            crate::player::Repeat::Off => "Off",
            crate::player::Repeat::One => "One",
            crate::player::Repeat::All => "All",
        };
        let quality = if s.stream_max_bitrate == 0 {
            "Original".to_string()
        } else {
            format!("{} kbps", s.stream_max_bitrate)
        };
        let fixed = if s.fixed_pipeline_rate {
            format!("{} Hz", s.fixed_pipeline_hz)
        } else {
            "Off".to_string()
        };
        let format = s
            .stream_format
            .clone()
            .unwrap_or_else(|| "Original".to_string());
        vec![
            (
                "Shuffle".into(),
                yes_no(self.player.shuffle()),
                SettingsRowAction::Shuffle,
            ),
            ("Repeat".into(), repeat.into(), SettingsRowAction::Repeat),
            (
                "Crossfade".into(),
                yes_no(self.player.crossfade()),
                SettingsRowAction::Crossfade,
            ),
            (
                "Crossfade length".into(),
                format!("{:.0}s", s.crossfade_seconds),
                SettingsRowAction::Length,
            ),
            (
                "Crossfade curve".into(),
                s.crossfade_curve.clone(),
                SettingsRowAction::Curve,
            ),
            (
                "Album-only crossfade".into(),
                yes_no(s.crossfade_album_only),
                SettingsRowAction::AlbumOnly,
            ),
            (
                "Normalization".into(),
                format!("{:?}", self.player.normalization()),
                SettingsRowAction::Normalize,
            ),
            (
                "Equalizer".into(),
                self.eq_profile.clone().unwrap_or_else(|| "Off".into()),
                SettingsRowAction::Eq,
            ),
            (
                "Palette contrast".into(),
                signed_percent(s.color_contrast),
                SettingsRowAction::ColorContrast,
            ),
            (
                "Palette saturation".into(),
                signed_percent(s.color_saturation),
                SettingsRowAction::ColorSaturation,
            ),
            (
                "Scrobbling".into(),
                yes_no(s.scrobble),
                SettingsRowAction::Scrobble,
            ),
            (
                "Bit-perfect DSP".into(),
                yes_no(s.bit_perfect),
                SettingsRowAction::BitPerfect,
            ),
            (
                "Exclusive access".into(),
                yes_no(s.exclusive),
                SettingsRowAction::Exclusive,
            ),
            (
                "Playback cache".into(),
                yes_no(s.stream_cache),
                SettingsRowAction::Cache,
            ),
            ("Stream quality".into(), quality, SettingsRowAction::Quality),
            ("Stream format".into(), format, SettingsRowAction::Format),
            (
                "Fixed pipeline rate".into(),
                fixed,
                SettingsRowAction::FixedRate,
            ),
            (
                "Sleep timer".into(),
                "30 minutes".into(),
                SettingsRowAction::Sleep30,
            ),
            (
                "Sleep timer".into(),
                "Off".into(),
                SettingsRowAction::SleepOff,
            ),
            ("".into(), "Close".into(), SettingsRowAction::Close),
        ]
    }

    pub(in crate::ui) fn settings_rows(&self) -> Vec<(String, String)> {
        self.settings_actions()
            .into_iter()
            .map(|(label, value, _)| (label, value))
            .collect()
    }

    /// `h`/`l` on the selected row. Numeric rows change in place rather than
    /// cycling through a fixed ladder; contrast uses a 0.1 step over the full
    /// -1..=1 range.
    pub(in crate::ui) fn adjust_settings_row(&self, direction: i8) {
        let Some((_, _, action)) = self
            .settings_actions()
            .into_iter()
            .nth(self.settings_cursor)
        else {
            return;
        };
        match action {
            SettingsRowAction::ColorContrast => {
                let next = contrast_after_step(self.settings.color_contrast, direction);
                self.bridge.set_color_contrast(next);
            }
            SettingsRowAction::ColorSaturation => {
                let next = contrast_after_step(self.settings.color_saturation, direction);
                self.bridge.set_color_saturation(next);
            }
            SettingsRowAction::Length => {
                let next =
                    (self.settings.crossfade_seconds + f64::from(direction)).clamp(1.0, 12.0);
                self.bridge
                    .set_crossfade_change(crate::playback_settings::CrossfadeChange::Length(next));
            }
            SettingsRowAction::Shuffle
            | SettingsRowAction::Repeat
            | SettingsRowAction::Crossfade
            | SettingsRowAction::Curve
            | SettingsRowAction::AlbumOnly
            | SettingsRowAction::Normalize
            | SettingsRowAction::Eq
            | SettingsRowAction::Scrobble
            | SettingsRowAction::BitPerfect
            | SettingsRowAction::Exclusive
            | SettingsRowAction::Cache
            | SettingsRowAction::Quality
            | SettingsRowAction::FixedRate
            | SettingsRowAction::Format
            | SettingsRowAction::Sleep30
            | SettingsRowAction::SleepOff
            | SettingsRowAction::Close => {}
        }
    }

    fn apply_settings_row(&mut self) {
        let Some((_, _, action)) = self
            .settings_actions()
            .into_iter()
            .nth(self.settings_cursor)
        else {
            return;
        };
        match action {
            SettingsRowAction::Shuffle => self.player.shuffle_queue(),
            SettingsRowAction::Repeat => self.player.cycle_repeat(),
            SettingsRowAction::Crossfade => self.player.toggle_crossfade(),
            SettingsRowAction::Length => {
                let next = match self.settings.crossfade_seconds.round() as u32 {
                    0..=3 => 5.0,
                    4..=5 => 8.0,
                    6..=8 => 12.0,
                    _ => 3.0,
                };
                self.bridge
                    .set_crossfade_change(crate::playback_settings::CrossfadeChange::Length(next));
            }
            SettingsRowAction::Curve => {
                let equal = self.settings.crossfade_curve != "linear";
                self.bridge
                    .set_crossfade_change(crate::playback_settings::CrossfadeChange::Curve(!equal));
            }
            SettingsRowAction::AlbumOnly => {
                let enabled = !self.settings.crossfade_album_only;
                self.bridge.set_crossfade_change(
                    crate::playback_settings::CrossfadeChange::AlbumOnly(enabled),
                );
            }
            SettingsRowAction::Normalize => self.player.cycle_normalization(),
            SettingsRowAction::Eq => {
                self.bridge.cycle_eq();
            }
            SettingsRowAction::ColorContrast => {
                const STEPS: [f64; 11] =
                    [-1.0, -0.8, -0.6, -0.4, -0.2, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
                let next = STEPS
                    .iter()
                    .copied()
                    .find(|step| *step > self.settings.color_contrast + 0.01)
                    .unwrap_or(STEPS[0]);
                self.bridge.set_color_contrast(next);
            }
            SettingsRowAction::ColorSaturation => {
                const STEPS: [f64; 11] =
                    [-1.0, -0.8, -0.6, -0.4, -0.2, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
                let next = STEPS
                    .iter()
                    .copied()
                    .find(|step| *step > self.settings.color_saturation + 0.01)
                    .unwrap_or(STEPS[0]);
                self.bridge.set_color_saturation(next);
            }
            SettingsRowAction::Scrobble => {
                self.bridge.set_scrobble(!self.settings.scrobble);
            }
            SettingsRowAction::BitPerfect => {
                self.bridge
                    .set_output_mode(None, Some(!self.settings.bit_perfect));
            }
            SettingsRowAction::Exclusive => {
                self.bridge
                    .set_output_mode(Some(!self.settings.exclusive), None);
            }
            SettingsRowAction::Cache => {
                self.bridge
                    .set_stream_change(crate::playback_settings::StreamChange::Cache(
                        !self.settings.stream_cache,
                    ));
            }
            SettingsRowAction::Quality => {
                let next = match self.settings.stream_max_bitrate {
                    0 => 128,
                    128 => 320,
                    _ => 0,
                };
                self.bridge
                    .set_stream_change(crate::playback_settings::StreamChange::Quality(next));
            }
            SettingsRowAction::Format => {
                let next = match self.settings.stream_format.as_deref() {
                    None => Some("mp3".to_string()),
                    Some("mp3") => Some("opus".to_string()),
                    _ => None,
                };
                self.bridge
                    .set_stream_change(crate::playback_settings::StreamChange::Format(next));
            }
            SettingsRowAction::FixedRate => {
                let next = match self.settings.fixed_pipeline_hz {
                    _ if !self.settings.fixed_pipeline_rate => Some(48_000),
                    44_100..=48_000 => Some(96_000),
                    48_001..=96_000 => None,
                    _ => Some(48_000),
                };
                self.bridge
                    .set_stream_change(crate::playback_settings::StreamChange::FixedRate(next));
            }
            SettingsRowAction::Sleep30 => {
                self.bridge.set_sleep_timer(Some(30));
            }
            SettingsRowAction::SleepOff => {
                self.bridge.set_sleep_timer(None);
            }
            SettingsRowAction::Close => self.settings_open = false,
        }
    }

    pub(in crate::ui) fn on_eq_cycle(
        &mut self,
        _: &EqCycle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.bridge.cycle_eq();
        cx.notify();
    }

    pub(in crate::ui) fn on_repeat_cycle(
        &mut self,
        _: &RepeatCycle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The core owns the repeat mode; the next playback state updates the
        // status bar badge.
        self.player.cycle_repeat();
        cx.notify();
    }

    pub(in crate::ui) fn on_queue_toggle(
        &mut self,
        _: &QueueToggle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_focus = None;
        self.queue_open = !self.queue_open;
        if self.queue_open {
            self.land_queue(self.player.index());
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_queue_remove(
        &mut self,
        _: &QueueRemove,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            self.delete_queue_range(cx);
        }
    }

    fn rows_word(count: usize) -> &'static str {
        if count == 1 { "row" } else { "rows" }
    }

    /// Replace the row clipboard from a row list, preserving order and
    /// duplicates. Returns the number of ids stored; an unsupported row kind
    /// (playlists) or an empty id skips that row.
    fn clipboard_from_rows(&mut self, rows: Vec<Row>) -> usize {
        let Some((kind, ids)) = clipboard_ids_from_rows(rows) else {
            return 0;
        };
        let count = ids.len();
        self.clipboard_kind = Some(kind);
        self.clipboard_ids = ids;
        count
    }

    fn clipboard_from_song_ids(&mut self, ids: Vec<String>) -> usize {
        let ids: Vec<String> = ids.into_iter().filter(|id| !id.is_empty()).collect();
        if ids.is_empty() {
            return 0;
        }
        let count = ids.len();
        self.clipboard_kind = Some(RowKind::Song);
        self.clipboard_ids = ids;
        count
    }

    /// The active surface's current selection as one row kind plus its ids,
    /// for `playlist add selected; <playlist>`. Mirrors `yank_surface`: the
    /// floating search wins, then the queue, then a page, then the library.
    /// The visual range is used when a selection is active; otherwise the
    /// cursor row counts as a one-row selection, like `yank`/`delete_row`.
    /// Playlist rows are skipped because a playlist cannot be appended by id.
    pub(in crate::ui) fn selected_playlist_ids(&self) -> Option<(&'static str, Vec<String>)> {
        if self.fullscreen || self.login.is_some() || self.command_line.is_open() {
            return None;
        }
        if self.search_open
            && let Some(search) = self.search.as_ref()
        {
            let scoped = search.scoped();
            let rows: Vec<Row> = search
                .selection
                .indices(search.cursor(), false)
                .into_iter()
                .filter_map(|index| scoped.get(index).map(|row| (*row).clone()))
                .collect();
            return clipboard_ids_from_rows(rows)
                .and_then(|(kind, ids)| kind.clipboard_kind().map(|kind| (kind, ids)));
        }
        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let ids: Vec<String> = self
                .queue_list
                .selection
                .indices(self.queue_list.cursor, false)
                .into_iter()
                .filter_map(|index| queue.get(index).map(|track| track.id.clone()))
                .filter(|id| !id.is_empty())
                .collect();
            return (!ids.is_empty()).then_some(("song", ids));
        }
        let rows: Vec<Row> = if let Some(page) = self.page.as_ref() {
            page.list
                .selection
                .indices(page.list.cursor, false)
                .into_iter()
                .filter_map(|index| page.rows.get(index).cloned())
                .collect()
        } else {
            self.view
                .list
                .selection
                .indices(self.view.list.cursor, false)
                .into_iter()
                .filter_map(|index| self.view.rows.row(index).cloned())
                .collect()
        };
        clipboard_ids_from_rows(rows)
            .and_then(|(kind, ids)| kind.clipboard_kind().map(|kind| (kind, ids)))
    }

    fn announce_yank(&mut self, count: usize, missing: usize) {
        if count == 0 {
            self.push_toast("Nothing to yank".to_string(), None);
            return;
        }
        let suffix = if missing > 0 {
            format!(" ({missing} not loaded)")
        } else {
            String::new()
        };
        self.push_toast(
            format!("Yanked {count} {}{suffix}", Self::rows_word(count)),
            None,
        );
    }

    /// `y` (range or cursor) / `Y` (cursor only) against the active surface.
    fn yank_surface(&mut self, row_only: bool) -> usize {
        if self.fullscreen || self.login.is_some() || self.command_line.is_open() {
            return 0;
        }
        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let indices = self
                .queue_list
                .selection
                .indices(self.queue_list.cursor, row_only);
            let ids: Vec<String> = indices
                .iter()
                .filter_map(|index| queue.get(*index).map(|track| track.id.clone()))
                .collect();
            let count = self.clipboard_from_song_ids(ids);
            self.queue_list.selection.clear();
            self.visual_mode = false;
            self.announce_yank(count, 0);
            return count;
        }
        let page_rows = self.page.as_ref().map(|page| {
            let indices = page.list.selection.indices(page.list.cursor, row_only);
            indices
                .iter()
                .filter_map(|index| page.rows.get(*index).cloned())
                .collect::<Vec<Row>>()
        });
        if let Some(rows) = page_rows {
            let count = self.clipboard_from_rows(rows);
            if let Some(page) = self.page.as_mut() {
                page.list.selection.clear();
            }
            self.visual_mode = false;
            self.announce_yank(count, 0);
            return count;
        }
        let indices = self
            .view
            .list
            .selection
            .indices(self.view.list.cursor, row_only);
        let selected = indices.len();
        let rows: Vec<Row> = indices
            .iter()
            .filter_map(|index| self.view.rows.get(*index).cloned())
            .collect();
        let missing = selected.saturating_sub(rows.len());
        let count = self.clipboard_from_rows(rows);
        self.view.list.selection.clear();
        self.visual_mode = false;
        self.announce_yank(count, missing);
        count
    }

    pub(in crate::ui) fn on_yank(
        &mut self,
        _: &Yank,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.yank_surface(false);
        cx.notify();
    }

    pub(in crate::ui) fn on_yank_row(
        &mut self,
        _: &YankRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.yank_surface(true);
        cx.notify();
    }

    /// Yank from the floating search's current scope (normal mode only; the
    /// typing mode keeps `y` as text input).
    pub(in crate::ui) fn yank_search(&mut self, row_only: bool) -> usize {
        let Some(search) = self.search.as_ref() else {
            return 0;
        };
        let rows = search.scoped().into_iter().cloned().collect::<Vec<Row>>();
        let indices = search.selection.indices(search.cursor(), row_only);
        let selected = indices.len();
        let picked: Vec<Row> = indices
            .iter()
            .filter_map(|index| rows.get(*index).cloned())
            .collect();
        let missing = selected.saturating_sub(picked.len());
        let count = self.clipboard_from_rows(picked);
        if let Some(search) = self.search.as_mut() {
            search.selection.clear();
        }
        self.visual_mode = false;
        self.announce_yank(count, missing);
        count
    }

    fn paste_clipboard(&mut self, above: bool, cx: &mut Context<Self>) {
        if self.fullscreen || self.login.is_some() {
            return;
        }
        let Some(kind) = self.clipboard_kind else {
            return;
        };
        let Some(kind_str) = kind.clipboard_kind() else {
            return;
        };
        let ids = std::mem::take(&mut self.clipboard_ids);
        if ids.is_empty() {
            self.clipboard_kind = None;
            return;
        }
        let target = if self.queue_open {
            Some(PasteTarget::Queue(paste_at(self.queue_list.cursor, above)))
        } else {
            self.page
                .as_ref()
                .filter(|page| page.descriptor.kind == PageKind::Playlist)
                .map(|page| {
                    PasteTarget::Playlist(
                        page.descriptor.id.clone(),
                        paste_at(page.list.cursor, above),
                    )
                })
        };
        let Some(target) = target else {
            // No valid target: keep the clipboard for the next attempt.
            self.clipboard_ids = ids;
            self.push_toast(
                "Open the queue or a playlist page to paste".to_string(),
                None,
            );
            cx.notify();
            return;
        };
        // One-shot clipboard: the same yanked rows cannot be pasted twice.
        // Keep a backup so a failed paste can restore it (Event::Error).
        self.clipboard_kind = None;
        self.paste_backup = Some((kind, ids.clone()));
        match target {
            PasteTarget::Queue(at) => {
                self.queue_list.selection.clear();
                self.visual_mode = false;
                self.bridge.paste_queue(kind_str.to_string(), ids, at);
            }
            PasteTarget::Playlist(playlist_id, position) => {
                self.bridge
                    .paste_playlist(playlist_id, kind_str.to_string(), ids, position);
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_paste_below(
        &mut self,
        _: &PasteBelow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_clipboard(false, cx);
    }

    pub(in crate::ui) fn on_paste_above(
        &mut self,
        _: &PasteAbove,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_clipboard(true, cx);
    }

    fn delete_queue_range(&mut self, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .queue_list
            .selection
            .indices(self.queue_list.cursor, false)
            .into_iter()
            .filter(|index| *index < self.player.len())
            .collect();
        if indices.is_empty() {
            return;
        }
        let first = indices[0];
        let count = self.player.remove_many(&indices);
        // The core is the authority: it shrinks `len()` when the delta lands.
        // Clamp against the expected remaining length so the cursor never
        // parks past the end in the meantime.
        let remaining = self.player.len().saturating_sub(count);
        self.queue_list.cursor = first.min(remaining.saturating_sub(1));
        self.queue_list.selection.clear();
        self.visual_mode = false;
        self.follow_queue(true);
        self.push_toast(format!("Deleted {count} {}", Self::rows_word(count)), None);
        cx.notify();
    }

    fn delete_playlist_range(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let indices: Vec<usize> = page
            .list
            .selection
            .indices(page.list.cursor, false)
            .into_iter()
            .filter(|index| *index < page.rows.len())
            .collect();
        if indices.is_empty() {
            return;
        }
        let first = indices[0];
        page.list.cursor = first;
        page.list.selection.clear();
        let playlist_id = page.descriptor.id.clone();
        self.visual_mode = false;
        self.bridge.delete_playlist_positions(playlist_id, indices);
        cx.notify();
    }

    /// `d d`: the queue range wins when the drawer is open, otherwise the
    /// playlist page edits its entries; other surfaces are inert.
    pub(in crate::ui) fn on_delete_row(
        &mut self,
        _: &DeleteRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fullscreen || self.login.is_some() {
            return;
        }
        if self.queue_open {
            self.delete_queue_range(cx);
            return;
        }
        if self
            .page
            .as_ref()
            .is_some_and(|page| page.descriptor.kind == PageKind::Playlist)
        {
            self.delete_playlist_range(cx);
        }
    }

    pub(in crate::ui) fn on_queue_up(
        &mut self,
        _: &QueueUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            if self.queue_list.selection.active() {
                self.move_queue_block(-1, cx);
            } else if self.queue_list.cursor > 0 {
                self.player
                    .move_item(self.queue_list.cursor, self.queue_list.cursor - 1);
                self.queue_list.cursor -= 1;
                self.follow_queue(true);
                cx.notify();
            }
            return;
        }
        self.move_playlist_rows(-1, cx);
    }

    /// Queue `Ctrl-Shift-K/J` with a visual selection: move the whole block.
    fn move_queue_block(&mut self, delta: i32, cx: &mut Context<Self>) {
        let anchor = self
            .queue_list
            .selection
            .anchor()
            .unwrap_or(self.queue_list.cursor);
        let start = anchor.min(self.queue_list.cursor);
        let end = anchor.max(self.queue_list.cursor);
        let len = end - start + 1;
        if !self.player.move_block(start, len, delta) {
            return;
        }
        self.queue_list.cursor = (self.queue_list.cursor as i64 + i64::from(delta)) as usize;
        self.queue_list.selection.shift(delta);
        self.follow_queue(true);
        cx.notify();
    }

    /// Playlist-page `Ctrl-Shift-K/J`: move the cursor entry or the whole
    /// selected block one position.
    fn move_playlist_rows(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if page.descriptor.kind != PageKind::Playlist {
            return;
        }
        let indices = page.list.selection.indices(page.list.cursor, false);
        let Some(&start) = indices.first() else {
            return;
        };
        let len = indices.len();
        let new_start = start as i64 + i64::from(delta);
        if new_start < 0
            || new_start as usize + len > page.rows.len()
            || new_start as usize == start
        {
            return;
        }
        let cursor = (page.list.cursor as i64 + i64::from(delta)) as usize;
        if page.list.selection.active() {
            page.list.selection.shift(delta);
        }
        page.list.cursor = cursor;
        let playlist_id = page.descriptor.id.clone();
        self.visual_mode = page.list.selection.active();
        self.bridge
            .move_playlist_block(playlist_id, start, len, delta, cursor);
        cx.notify();
    }

    pub(in crate::ui) fn on_queue_down(
        &mut self,
        _: &QueueDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            if self.queue_list.selection.active() {
                self.move_queue_block(1, cx);
            } else if self.queue_list.cursor + 1 < self.player.len() {
                self.player
                    .move_item(self.queue_list.cursor, self.queue_list.cursor + 1);
                self.queue_list.cursor += 1;
                self.follow_queue(true);
                cx.notify();
            }
            return;
        }
        self.move_playlist_rows(1, cx);
    }

    pub(in crate::ui) fn on_play_pause(
        &mut self,
        _: &PlayPause,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.press_transport(1);
        self.player.toggle();
        cx.notify();
    }

    pub(in crate::ui) fn on_mute(
        &mut self,
        _: &Mute,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Show the sticky bubble on the expected state immediately; the next
        // state frame re-applies `sync_mute_toast` with the core's truth.
        let muted = !self.player.muted();
        self.player.toggle_mute();
        self.sync_mute_toast(muted);
        cx.notify();
    }

    pub(in crate::ui) fn on_next_track(
        &mut self,
        _: &NextTrack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_command("track next", cx);
    }

    pub(in crate::ui) fn execute_track_next(&mut self, cx: &mut Context<Self>) {
        // The core answers asynchronously, so `player.index()` is still the
        // pre-IPC row here. Land the queue cursor on the locally computed
        // target; the authoritative index replaces it when the state frame
        // arrives (and a shuffle target is only known to the core, so the
        // cursor stays put until then).
        let target = local_track_step(
            self.player.index(),
            self.player.len(),
            self.player.repeat(),
            self.player.shuffle(),
            TrackStep::Next,
        );
        self.press_transport(2);
        self.player.next();
        if self.queue_open
            && let Some(index) = target
        {
            self.land_queue(index);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_prev_track(
        &mut self,
        _: &PrevTrack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_command("track previous", cx);
    }

    pub(in crate::ui) fn execute_track_previous(&mut self, cx: &mut Context<Self>) {
        let target = local_track_step(
            self.player.index(),
            self.player.len(),
            self.player.repeat(),
            self.player.shuffle(),
            TrackStep::Previous,
        );
        self.press_transport(0);
        self.player.previous();
        if self.queue_open
            && let Some(index) = target
        {
            self.land_queue(index);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_search_scope_next(
        &mut self,
        _: &SearchScopeNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_search.open {
            self.line_search_step(1);
            cx.notify();
            return;
        }
        if self.search.is_some() && self.search_open {
            self.move_search_scope(1);
            cx.notify();
            return;
        }
        self.on_tab(false, cx);
    }

    pub(in crate::ui) fn on_search_scope_prev(
        &mut self,
        _: &SearchScopePrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_search.open {
            self.line_search_step(-1);
            cx.notify();
            return;
        }
        if self.search.is_some() && self.search_open {
            self.move_search_scope(-1);
            cx.notify();
            return;
        }
        self.on_tab(true, cx);
    }

    /// Whether Tab may walk the fullscreen cover/title/lyrics panes. The
    /// lyrics overlay owns the keyboard while it is open, so the renderer
    /// hides the player's focus chrome for exactly this window.
    pub(in crate::ui) fn fullscreen_focus_walk_available(&self) -> bool {
        !self.lyrics_overlay
    }

    /// Tab's contextual walk. The superior bar exposes the current page's
    /// chips (the artist entry adds Download/Shuffle/Favorite) followed by
    /// the status actions; leaving the end drops back to the content.
    pub(in crate::ui) fn on_tab(&mut self, backward: bool, cx: &mut Context<Self>) {
        if self.fullscreen {
            if self.fullscreen_focus_walk_available() {
                self.cycle_fullscreen_focus(backward);
            }
            cx.notify();
            return;
        }
        let order = self.topbar_tab_order();
        self.status_focus = next_tab_slot(&order, self.status_focus, backward);
        cx.notify();
    }

    pub(in crate::ui) fn on_page_down(
        &mut self,
        _: &PageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.description.is_some() {
            self.page_description(true);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() && !self.queue_open {
            self.scroll_lyrics_by(self.lyrics_page_step(), cx);
            cx.notify();
            return;
        }
        // A list page motion pages the queue too while its overlay owns the
        // surface (U-24: it used to page the list hidden behind it). The step
        // is the number of queue rows the measured viewport paints.
        if self.queue_open {
            self.queue_list.cursor = self
                .queue_list
                .cursor
                .saturating_add(queue_page_rows(&self.queue_scroll_modal))
                .min(self.player.len().saturating_sub(1));
            self.hovered_row = None;
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.select(page.list.cursor.saturating_add(LIST_PAGE_ROWS));
            self.follow_page(true);
        } else {
            self.select(self.view.list.cursor.saturating_add(LIST_PAGE_ROWS));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_page_up(
        &mut self,
        _: &PageUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.description.is_some() {
            self.page_description(false);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() && !self.queue_open {
            self.scroll_lyrics_by(-self.lyrics_page_step(), cx);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_list.cursor = self
                .queue_list
                .cursor
                .saturating_sub(queue_page_rows(&self.queue_scroll_modal));
            self.hovered_row = None;
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.select(page.list.cursor.saturating_sub(LIST_PAGE_ROWS));
            self.follow_page(true);
        } else {
            self.select(self.view.list.cursor.saturating_sub(LIST_PAGE_ROWS));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_visual(
        &mut self,
        _: &Visual,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            let active = self.queue_list.selection.active();
            self.visual_mode = !active;
            if active {
                self.queue_list.selection.clear();
            } else {
                self.queue_list.selection.start(self.queue_list.cursor);
            }
            self.hovered_row = None;
            cx.notify();
            return;
        }
        let (active, cursor) = if let Some(page) = self.page.as_ref() {
            (page.list.selection.active(), page.list.cursor)
        } else {
            (self.view.list.selection.active(), self.view.list.cursor)
        };
        self.visual_mode = !active;
        if active {
            if let Some(page) = self.page.as_mut() {
                page.list.selection.clear();
            } else {
                self.view.list.selection.clear();
            }
        } else if let Some(page) = self.page.as_mut() {
            page.list.selection.start(cursor);
        } else {
            self.view.list.selection.start(cursor);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_visual_all(
        &mut self,
        _: &VisualAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visual_mode = true;
        if self.queue_open {
            self.queue_list.selection.select_all();
            self.queue_list.cursor = self.player.len().saturating_sub(1);
            self.follow_queue(false);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.list.selection.select_all();
            let last = page.rows.len().saturating_sub(1);
            page.select(last);
        } else {
            self.view.list.selection.select_all();
            let last = self.view.rows.len().saturating_sub(1);
            self.select(last);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_enter(
        &mut self,
        _: &Enter,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_open {
            self.apply_settings_row();
            cx.notify();
            return;
        }
        if self.line_search.open {
            self.line_search_commit(_window, cx);
            return;
        }
        if self.alt.armed && self.alt.sticky {
            let from_search = self.search_open
                && self.search.is_some()
                && self.alt_target.is_some()
                && self.status_focus.is_none();
            let menu = self.alt_menu.clone();
            let mut acted = false;
            if let Some(menu) = menu
                && let Some(verb) = self.alt.confirm_plan(&menu)
            {
                self.dispatch_quick(verb);
                acted = true;
            }
            if acted && from_search {
                self.dismiss_search_after_quick();
            }
            self.alt.cancel();
            self.alt_menu = None;
            self.alt_target = None;
            self.animate_alt(0.0);
            cx.notify();
            return;
        }
        if let Some(slot) = self.status_focus {
            self.activate_status(slot);
            cx.notify();
            return;
        }
        if self.search.is_some() {
            self.activate_search_result(cx);
            return;
        }
        if self.queue_open {
            let index = self.queue_list.cursor;
            self.player.jump(index);
            self.queue_open = false;
            cx.notify();
            return;
        }
        if self.page.is_some() {
            self.activate_page_cursor();
        } else {
            let row = self.view.rows.get(self.view.list.cursor).cloned();
            if let Some(row) = row {
                self.activate_row(&row);
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_back(
        &mut self,
        _: &Back,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // U-12: the double-`q` quit window is measured against `clock.now`.
        self.clock.tick();
        // Back is the Escape/cancel path: every branch that consumes a surface
        // also ends the visual range, so the next list action starts from a
        // deterministic resting state (U-24).
        if self.settings_open {
            self.settings_open = false;
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.line_search.open {
            self.close_line_search();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.alt.armed || self.alt_menu.is_some() {
            self.alt.cancel();
            self.alt_menu = None;
            self.alt_target = None;
            self.animate_alt(0.0);
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.status_focus.is_some() {
            self.status_focus = None;
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.description.is_some() {
            self.description = None;
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.lyrics_overlay {
            self.lyrics_overlay = false;
            self.lyrics_overlay_wanted = false;
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.fullscreen || self.fullscreen_anim.value(&self.clock) > 0.01 {
            self.fullscreen = false;
            self.release_lyrics_hold();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.queue_open && self.queue_list.selection.active() {
            self.queue_list.selection.clear();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.queue_open || self.queue_anim.value(&self.clock) > 0.01 {
            // Closing the drawer keeps a half-painted selection alive
            // otherwise; the list behind is the new focus.
            self.queue_open = false;
            self.queue_list.selection.clear();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.search.is_some() {
            self.search_open = false;
            if let Some(search) = self.search.as_mut() {
                search.selection.clear();
            }
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self
            .page
            .as_ref()
            .is_some_and(|page| page.list.selection.active())
        {
            if let Some(page) = self.page.as_mut() {
                page.list.selection.clear();
            }
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.view.list.selection.active() && self.page.is_none() {
            self.view.list.selection.clear();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.page.is_some() {
            self.back_page();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        let now = self.clock.now;
        if self.quit_armed_at.is_some_and(|at| now - at <= 0.25) {
            self.quit_armed_at = None;
            self.exiting = Some(now);
            cx.notify();
            return;
        }
        self.quit_armed_at = Some(now);
        cx.notify();
    }

    pub(in crate::ui) fn on_prev_place(
        &mut self,
        _: &PrevPlace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_travel(-1);
        cx.notify();
    }

    pub(in crate::ui) fn on_next_place(
        &mut self,
        _: &NextPlace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_travel(1);
        cx.notify();
    }

    pub(in crate::ui) fn on_home(
        &mut self,
        _: &Home,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_line_search();
        self.alt.cancel();
        self.alt_menu = None;
        self.alt_target = None;
        self.animate_alt(0.0);
        // Home is the deterministic "back to the resting list" reset: every
        // transient overlay and the visual range must go with it (U-24 left
        // settings/visual mode/queue selection behind).
        self.settings_open = false;
        self.settings_cursor = 0;
        self.search_open = false;
        if let Some(search) = self.search.as_mut() {
            search.selection.clear();
        }
        self.search = None;
        self.queue_open = false;
        self.fullscreen = false;
        self.lyrics_overlay = false;
        self.lyrics_overlay_wanted = false;
        self.release_lyrics_hold();
        self.description = None;
        if let Some(page) = self.page.as_mut() {
            page.list.selection.clear();
        }
        self.page = None;
        self.travel = Travel::default();
        self.visual_mode = false;
        self.queue_list.selection.clear();
        self.view.list.selection.clear();
        self.hovered_row = None;
        self.hovered_page = None;
        cx.notify();
    }

    /// The painted height of the description reader's scroll body.
    fn description_viewport(&self) -> f32 {
        let height = f32::from(self.description_scroll.bounds().size.height);
        if height > 1.0 {
            height
        } else {
            (self.win_h * 0.6).max(1.0)
        }
    }

    /// Scroll the description reader by `delta` pixels (positive = down),
    /// clamped to the real content range tracked by the scroll handle.
    fn scroll_description(&self, delta: f32) {
        // The handle stores the offset negatively (content moved up); the
        // tracked `max_offset` is the positive scroll range.
        let current = -f32::from(self.description_scroll.offset().y);
        let max = f32::from(self.description_scroll.max_offset().height);
        let next = description_offset(current, delta, max);
        self.description_scroll
            .set_offset(gpui::point(px(0.0), px(-next)));
    }

    /// Page the description reader: 90% of the visible height per motion.
    fn page_description(&self, down: bool) {
        let step = self.description_viewport() * DESCRIPTION_PAGE_RATIO;
        self.scroll_description(if down { step } else { -step });
    }
}

/// Rows one PageUp/PageDown motion crosses in the queue overlay: the rows the
/// measured viewport paints, so a short window pages little and a tall one
/// pages more (a hard-coded step would scroll past rows the user never saw).
fn queue_page_rows(scroll: &scroll_modal::ScrollModal) -> usize {
    let height = scroll.viewport_height();
    if height <= 1.0 {
        return LIST_PAGE_ROWS;
    }
    ((height / scroll.row_h).floor() as usize).max(1)
}

/// Which local transport step to predict for the queue cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackStep {
    Next,
    Previous,
}

/// Predicts the row the core will land on after `track next`/`track previous`
/// so the queue cursor can move in the same frame as the key press.
///
/// Only the deterministic linear order is predicted: shuffle picks from the
/// core's private plan (and previous walks its play history), so those keep the
/// cursor where it is and let the authoritative state frame land it.
fn local_track_step(
    index: usize,
    len: usize,
    repeat: crate::player::Repeat,
    shuffle: bool,
    step: TrackStep,
) -> Option<usize> {
    if len == 0 || shuffle {
        return None;
    }
    let index = index.min(len - 1);
    match step {
        TrackStep::Next => {
            if index + 1 < len {
                Some(index + 1)
            } else if repeat == crate::player::Repeat::All {
                Some(0)
            } else {
                None
            }
        }
        TrackStep::Previous => index.checked_sub(1),
    }
}

enum PasteTarget {
    Queue(usize),
    Playlist(String, usize),
}

/// Collect clipboard ids from already-materialized rows. The first supported
/// row fixes the kind; ids keep row order and duplicates. Playlist rows and
/// empty ids are skipped.
fn clipboard_ids_from_rows(rows: Vec<Row>) -> Option<(RowKind, Vec<String>)> {
    let mut kind = None;
    let mut ids = Vec::new();
    for row in rows {
        if row.kind.clipboard_kind().is_none() {
            continue;
        }
        let id = if row.kind == RowKind::Song {
            if row.song_id.is_empty() {
                row.item.id
            } else {
                row.song_id
            }
        } else {
            row.item.id
        };
        if id.is_empty() {
            continue;
        }
        match kind {
            None => {
                kind = Some(row.kind);
                ids.push(id);
            }
            Some(seen) if seen == row.kind => ids.push(id),
            _ => {}
        }
    }
    kind.map(|kind| (kind, ids))
}

/// `p` below / `P` above the queue cursor.
fn paste_at(cursor: usize, above: bool) -> usize {
    cursor.saturating_add(usize::from(!above))
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    fn row(kind: RowKind, id: &str, song_id: &str) -> Row {
        Row {
            item: crate::model::ItemMeta {
                id: id.to_string(),
                title: String::new(),
                cover_url: String::new(),
                cover_remote: String::new(),
                album_id: String::new(),
            },
            kind,
            subtitle: String::new(),
            meta: String::new(),
            track: String::new(),
            song_id: song_id.to_string(),
            starred: false,
            rating: 0,
        }
    }

    #[test]
    fn clipboard_keeps_order_duplicates_and_song_ids() {
        let rows = vec![
            row(RowKind::Song, "a", "s1"),
            row(RowKind::Song, "b", "s1"),
            row(RowKind::Song, "c", ""),
        ];
        assert_eq!(
            clipboard_ids_from_rows(rows),
            Some((
                RowKind::Song,
                vec!["s1".to_string(), "s1".to_string(), "c".to_string()]
            ))
        );
    }

    #[test]
    fn clipboard_uses_album_ids_and_rejects_playlists() {
        let rows = vec![
            row(RowKind::Album, "al1", ""),
            row(RowKind::Album, "al2", ""),
        ];
        assert_eq!(
            clipboard_ids_from_rows(rows),
            Some((RowKind::Album, vec!["al1".to_string(), "al2".to_string()]))
        );
        assert_eq!(
            clipboard_ids_from_rows(vec![row(RowKind::Playlist, "p1", "")]),
            None
        );
        assert_eq!(clipboard_ids_from_rows(Vec::new()), None);
    }

    #[test]
    fn paste_at_is_below_or_above_the_cursor() {
        assert_eq!(paste_at(0, false), 1);
        assert_eq!(paste_at(0, true), 0);
        assert_eq!(paste_at(3, false), 4);
        assert_eq!(paste_at(3, true), 3);
        assert_eq!(paste_at(usize::MAX, false), usize::MAX);
    }
}

#[cfg(test)]
mod contrast_step_tests {
    use super::{contrast_after_step, signed_percent};

    #[test]
    fn contrast_steps_in_both_directions_and_clamps() {
        assert!((contrast_after_step(0.0, 1) - 0.1).abs() < 1e-9);
        assert!((contrast_after_step(0.0, -1) + 0.1).abs() < 1e-9);
        assert!((contrast_after_step(0.4, 1) - 0.5).abs() < 1e-9);
        assert!((contrast_after_step(0.4, -1) - 0.3).abs() < 1e-9);
        assert!((contrast_after_step(0.95, 1) - 1.0).abs() < 1e-9);
        assert!((contrast_after_step(-0.95, -1) + 1.0).abs() < 1e-9);
    }

    #[test]
    fn contrast_labels_show_the_sign() {
        assert_eq!(signed_percent(0.4), "+40%");
        assert_eq!(signed_percent(-0.4), "-40%");
        assert_eq!(signed_percent(0.0), "0%");
    }
}

#[cfg(test)]
mod track_step_tests {
    use super::{TrackStep, local_track_step};
    use crate::player::Repeat;

    #[test]
    fn next_walks_forward_and_wraps_only_with_repeat_all() {
        assert_eq!(
            local_track_step(0, 4, Repeat::Off, false, TrackStep::Next),
            Some(1)
        );
        // Repeat One only affects auto-advance; an explicit next still moves.
        assert_eq!(
            local_track_step(0, 4, Repeat::One, false, TrackStep::Next),
            Some(1)
        );
        assert_eq!(
            local_track_step(3, 4, Repeat::All, false, TrackStep::Next),
            Some(0)
        );
        assert_eq!(
            local_track_step(3, 4, Repeat::Off, false, TrackStep::Next),
            None
        );
    }

    #[test]
    fn previous_clamps_at_the_head() {
        assert_eq!(
            local_track_step(2, 4, Repeat::Off, false, TrackStep::Previous),
            Some(1)
        );
        assert_eq!(
            local_track_step(0, 4, Repeat::Off, false, TrackStep::Previous),
            None
        );
    }

    #[test]
    fn shuffle_and_empty_queue_leave_the_cursor_to_the_core() {
        assert_eq!(
            local_track_step(1, 4, Repeat::All, true, TrackStep::Next),
            None
        );
        assert_eq!(
            local_track_step(1, 4, Repeat::All, true, TrackStep::Previous),
            None
        );
        assert_eq!(
            local_track_step(0, 0, Repeat::All, false, TrackStep::Next),
            None
        );
    }

    #[test]
    fn stale_out_of_range_index_is_clamped() {
        assert_eq!(
            local_track_step(9, 3, Repeat::All, false, TrackStep::Next),
            Some(0)
        );
        assert_eq!(
            local_track_step(9, 3, Repeat::All, false, TrackStep::Previous),
            Some(1)
        );
    }
}
