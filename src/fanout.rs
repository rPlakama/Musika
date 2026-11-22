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

use anyhow::Result;
use tokio::task::JoinSet;

use crate::consts::FETCH_CONCURRENCY;

/// Calls `on_result` for each id with at most [`FETCH_CONCURRENCY`] fetches in
/// flight, so callers can fold results without materialising the whole set.
///
/// The window slides (N-19): a finished fetch frees its slot for the next id
/// immediately instead of waiting for the slowest fetch of the current chunk,
/// so one slow request only delays the ids queued behind it.
pub async fn for_each_bounded<T, F, Fut, G>(ids: &[String], f: F, on_result: G)
where
    T: Send + 'static,
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    G: FnMut(String, Result<T>),
{
    for_each_windowed(ids, FETCH_CONCURRENCY, f, on_result).await;
}

/// [`for_each_bounded`] with an explicit window width.
///
/// The download pass runs much heavier work per id and stays polite to the
/// server with a smaller window ([`crate::consts::DOWNLOAD_CONCURRENCY`]).
pub async fn for_each_windowed<T, F, Fut, G>(ids: &[String], window: usize, f: F, mut on_result: G)
where
    T: Send + 'static,
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    G: FnMut(String, Result<T>),
{
    let window = window.max(1);
    let mut set = JoinSet::new();
    let mut queued = ids.iter();
    while set.len() < window {
        let Some(id) = queued.next() else { break };
        spawn_fetch(&mut set, id, &f);
    }
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((id, result)) => on_result(id, result),
            Err(err) => on_result(
                "<task>".to_string(),
                Err(anyhow::anyhow!("fetch task failed: {err}")),
            ),
        }
        // Refill the freed slot before waiting for the next completion: this is
        // the sliding window that keeps the pipeline full.
        if let Some(id) = queued.next() {
            spawn_fetch(&mut set, id, &f);
        }
    }
}

fn spawn_fetch<T, F, Fut>(set: &mut JoinSet<(String, Result<T>)>, id: &str, f: &F)
where
    T: Send + 'static,
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
{
    let id = id.to_string();
    let f = f.clone();
    set.spawn(async move {
        let result = f(id.clone()).await;
        (id, result)
    });
}

pub async fn map_bounded<T, F, Fut>(ids: Vec<String>, f: F) -> Vec<(String, Result<T>)>
where
    T: Send + 'static,
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
{
    let mut results = Vec::with_capacity(ids.len());
    for_each_bounded(&ids, f, |id, result| results.push((id, result))).await;
    results
}
