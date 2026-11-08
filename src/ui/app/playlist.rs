//! Selected-song extraction for `create_playlist`.

use super::super::*;

impl AppRoot {
    /// Song ids from the active surface, preserving the visible order.
    ///
    /// Visual selections are used when present; otherwise the current row is
    /// the selection. Album/artist/playlist rows are rejected rather than
    /// silently expanded, because the command is explicitly song-based.
    pub(in crate::ui) fn selected_song_ids(&self) -> Result<Vec<String>, String> {
        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let ids: Vec<Option<String>> =
                queue.iter().map(|track| Some(track.id.clone())).collect();
            return selection_ids(
                &ids,
                |index| self.queue_selection.contains(self.queue_cursor, index),
                self.queue_selection.active(),
                self.queue_cursor,
            )
            .map_err(|_| "Select a queue song first.".to_string());
        }
        if let Some(search) = self.search.as_ref() {
            let rows = search.scoped();
            let songs: Vec<Option<String>> = rows
                .iter()
                .map(|row| {
                    (row.kind == RowKind::Song)
                        .then(|| row.song_id.clone())
                        .filter(|id| !id.is_empty())
                })
                .collect();
            return selection_ids(
                &songs,
                |index| search.selection.contains(search.cursor(), index),
                search.selection.active(),
                search.cursor(),
            );
        }
        if let Some(page) = self.page.as_ref() {
            let songs: Vec<Option<String>> = page
                .rows
                .iter()
                .map(|row| {
                    (row.kind == RowKind::Song)
                        .then(|| row.song_id.clone())
                        .filter(|id| !id.is_empty())
                })
                .collect();
            return selection_ids(
                &songs,
                |index| page.selection.contains(page.cursor, index),
                page.selection.active(),
                page.cursor,
            );
        }
        let rows = &self.view.rows;
        let songs: Vec<Option<String>> = (0..rows.count)
            .map(|index| {
                rows.row(index).and_then(|row| {
                    (row.kind == RowKind::Song)
                        .then(|| row.song_id.clone())
                        .filter(|id| !id.is_empty())
                })
            })
            .collect();
        selection_ids(
            &songs,
            |index| self.view.selection.contains(self.view.cursor, index),
            self.view.selection.active(),
            self.view.cursor,
        )
    }
}

fn selection_ids(
    ids: &[Option<String>],
    selected: impl Fn(usize) -> bool,
    has_selection: bool,
    cursor: usize,
) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Err("There are no songs here.".to_string());
    }
    let mut out = Vec::new();
    if has_selection {
        for (index, id) in ids.iter().enumerate() {
            if !selected(index) {
                continue;
            }
            let id = id
                .as_ref()
                .ok_or_else(|| "Only song rows can be added to a playlist.".to_string())?;
            out.push(id.clone());
        }
    } else {
        let id = ids[cursor.min(ids.len().saturating_sub(1))]
            .as_ref()
            .ok_or_else(|| "The current row is not a song.".to_string())?;
        out.push(id.clone());
    }
    if out.is_empty() {
        return Err("Select a song first.".to_string());
    }
    Ok(out)
}
