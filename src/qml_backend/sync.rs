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

use std::sync::Arc;

use qtbridge::QmlMethodInvoker;

use crate::http_client::SubsonicClient;

use super::support::raise;

pub(super) fn index_value(value: i64) -> usize {
    value.max(0) as usize
}

/// Status-bar summary of a finished sync; lyrics only when any were fetched.
pub(super) fn sync_summary(report: &crate::index::SyncReport) -> String {
    let mut summary = format!(
        "{} artists, {} albums, {} songs",
        report.artists, report.albums, report.songs
    );
    if report.lyrics > 0 {
        summary.push_str(&format!(", {} lyrics", report.lyrics));
    }
    summary
}

pub(super) fn online_authenticated(
    invoker: &QmlMethodInvoker,
    sync_finish: QmlMethodInvoker,
    sync_progress: QmlMethodInvoker,
    client: SubsonicClient,
    kind: String,
    announce_login: bool,
) {
    qtbridge::invoke_method!(invoker, "noteServerType", kind);
    invoker.invoke_method("noteOnline");
    if announce_login {
        invoker.invoke_method("login_succeeded");
    }
    crate::debug_log!("login: online mode — server available");
    spawn_sync(sync_finish, sync_progress, client, true);
}

/// One background index sync: the single pipeline behind the manual button/
/// IPC, a fresh login and the cached auto-login (ARC-8).
///
/// `emit_started` lets synchronous callers (the manual slot) show "Syncing…"
/// on the keypress instead of one task hop later. Success always logs the
/// report, flips the offline flag back (CACHE-9) and finishes with the
/// summary; failure raises the shared toast.
pub(super) fn spawn_sync(
    finish_invoker: QmlMethodInvoker,
    progress_invoker: QmlMethodInvoker,
    client: SubsonicClient,
    emit_started: bool,
) {
    let Some(dbs) = crate::dbs() else {
        eprintln!("index: databases are not initialised");
        return;
    };

    crate::runtime().spawn(async move {
        if emit_started {
            qtbridge::invoke_method!(progress_invoker, "librarySyncStarted");
        }

        let client = Arc::new(client);
        let cover_client = Arc::clone(&client);
        let progress = move |update: crate::index::SyncProgress| {
            qtbridge::invoke_method!(
                progress_invoker,
                "librarySyncProgress",
                update.message,
                update.done as i64,
                update.total as i64
            );
        };

        match crate::index::sync_library(&dbs, client, progress).await {
            Ok(report) => {
                crate::debug_log!(
                    "index: library updated: {} artists, {} albums, {} songs, {} lyrics, {} playlists",
                    report.artists,
                    report.albums,
                    report.songs,
                    report.lyrics,
                    report.playlists
                );
                // CACHE-9: a successful sync means the server is back.
                qtbridge::invoke_method!(finish_invoker, "noteOnline");
                // The reload gate: a no-op sync (updated = false) finishes
                // without tearing the lists down.
                qtbridge::invoke_method!(
                    finish_invoker,
                    "librarySyncFinished",
                    sync_summary(&report),
                    report.updated
                );

                // Cover caching is a secondary task: the library sync is
                // already complete, so this never holds up the finish state.
                if report.covers_due {
                    let cover_dbs = dbs.clone();
                    let forced = report.forced_covers.clone();
                    crate::runtime().spawn(async move {
                        let result =
                            crate::index::cache_covers(&cover_dbs, &cover_client, &forced).await;
                        crate::debug_log!(
                            "index: cover cache: {} new, {} upgraded, {} unchanged, {} removed",
                            result.downloaded,
                            result.upgraded,
                            result.unchanged,
                            result.removed
                        );
                        let changed = result.downloaded > 0 || result.upgraded > 0;
                        qtbridge::invoke_method!(finish_invoker, "coversCached", changed);
                    });
                }
            }
            Err(err) => {
                eprintln!("index: library sync failed: {err:#}");
                qtbridge::invoke_method!(
                    finish_invoker,
                    "librarySyncFinished",
                    format!("Sync failed: {err:#}")
                );
                raise(&finish_invoker, "sync", format!("{err:#}"));
            }
        }
    });
}
