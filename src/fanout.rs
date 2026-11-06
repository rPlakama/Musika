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

/// Calls `on_result` for each id in bounded chunks (one
/// `FETCH_CONCURRENCY` window live at a time), so callers can fold results
/// without materialising the whole set.
pub async fn for_each_bounded<T, F, Fut, G>(ids: &[String], f: F, mut on_result: G)
where
    T: Send + 'static,
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    G: FnMut(String, Result<T>),
{
    for chunk in ids.chunks(FETCH_CONCURRENCY) {
        let mut set = JoinSet::new();
        for id in chunk {
            let id = id.clone();
            let f = f.clone();
            set.spawn(async move {
                let result = f(id.clone()).await;
                (id, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((id, result)) => on_result(id, result),
                Err(err) => on_result(
                    "<task>".to_string(),
                    Err(anyhow::anyhow!("fetch task failed: {err}")),
                ),
            }
        }
    }
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
