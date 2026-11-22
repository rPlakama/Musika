use super::common::*;
use crate::fanout::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[tokio::test]
async fn maps_every_id_and_collects_errors() {
    let ids: Vec<String> = (0..20).map(|id| id.to_string()).collect();
    let results = map_bounded(ids.clone(), |id| async move {
        if id == "7" {
            anyhow::bail!("failed");
        }
        Ok(id.parse::<usize>().unwrap())
    })
    .await;

    assert_eq!(results.len(), ids.len());
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for (id, result) in results {
        match result {
            Ok(value) => ok.push((id, value)),
            Err(_) => failed.push(id),
        }
    }
    assert_eq!(failed, vec!["7"]);
    assert_eq!(ok.len(), 19);
    assert!(ok.contains(&("19".to_string(), 19)));
}

/// A finished fetch must free its slot immediately: the slow id of the first
/// window is reported *after* ids from the next window, which only start once
/// earlier fast fetches complete. A full-join-per-chunk implementation reports
/// the slow id before any later-window id (head-of-line blocking).
#[tokio::test]
async fn window_slides_instead_of_waiting_for_the_slowest_fetch() {
    let window = crate::consts::FETCH_CONCURRENCY;
    let slow_id = 0usize;
    let ids: Vec<String> = (0..window * 2 + 2).map(|id| id.to_string()).collect();
    let completed = Arc::new(Mutex::new(Vec::new()));
    for_each_bounded(
        &ids,
        move |id| async move {
            let index = id.parse::<usize>()?;
            let delay = if index == slow_id { 200 } else { 2 };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            Ok::<usize, anyhow::Error>(index)
        },
        {
            let completed = Arc::clone(&completed);
            move |id, _result| {
                completed
                    .lock()
                    .expect("completed order lock")
                    .push(id.parse::<usize>().expect("numeric id"));
            }
        },
    )
    .await;

    let order = completed.lock().expect("completed order lock").clone();
    assert_eq!(order.len(), ids.len());
    let slow_position = order
        .iter()
        .position(|id| *id == slow_id)
        .expect("slow id reported");
    let next_window_position = order
        .iter()
        .position(|id| *id == window)
        .expect("next window id reported");
    assert!(
        next_window_position < slow_position,
        "next window must start before the slow fetch finishes: {order:?}"
    );
}

/// At most `window` fetches are live at any moment, and the window stays full
/// while work remains.
#[tokio::test]
async fn window_bounds_concurrency() {
    let window = 3;
    let ids: Vec<String> = (0..10).map(|id| id.to_string()).collect();
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    for_each_windowed(
        &ids,
        window,
        {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            move |id| {
                let in_flight = Arc::clone(&in_flight);
                let peak = Arc::clone(&peak);
                async move {
                    let live = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(live, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    Ok::<String, anyhow::Error>(id)
                }
            }
        },
        |_id, _result| {},
    )
    .await;

    assert_eq!(peak.load(Ordering::SeqCst), window);
    assert_eq!(in_flight.load(Ordering::SeqCst), 0);
}
