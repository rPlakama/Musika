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

//! Library-sync serialisation, pause on offline and replay on reconnect.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{Notify, mpsc, watch};

use crate::bridge::{Event, SyncMode, SyncQueue, drive_syncs};

/// A fake sync pass counts drops so the test can see the offline cancellation.
struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn queued_requests_keep_the_deepest_mode_and_start_once() {
    let queue = SyncQueue::new();

    assert!(queue.request(SyncMode::Incremental));
    assert!(!queue.request(SyncMode::All));
    assert!(!queue.request(SyncMode::Incremental));
    assert_eq!(queue.next(), Some((SyncMode::All, 0)));

    // The slot is released with the last claim, so a later request starts a
    // fresh driver instead of being stranded behind the finished one.
    assert_eq!(queue.next(), None);
    assert!(queue.request(SyncMode::Incremental));
    assert_eq!(queue.next(), Some((SyncMode::Incremental, 0)));
    assert_eq!(queue.next(), None);
}

#[tokio::test]
async fn offline_cancels_a_live_pass_and_replays_its_mode_on_reconnect() {
    let queue = Arc::new(SyncQueue::new());
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let (online_tx, online_rx) = watch::channel(true);
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let finish = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicUsize::new(0));

    let run_started = started_tx.clone();
    let run_finish = Arc::clone(&finish);
    let run_dropped = Arc::clone(&dropped);
    let driver = tokio::spawn(drive_syncs(
        Arc::clone(&queue),
        events_tx,
        online_rx,
        move |mode| {
            let started = run_started.clone();
            let finish = Arc::clone(&run_finish);
            let dropped = Arc::clone(&run_dropped);
            async move {
                started.send(mode).expect("run start channel");
                let guard = DropCounter(dropped);
                finish.notified().await;
                drop(guard);
                Ok(())
            }
        },
    ));

    assert!(queue.request(SyncMode::All));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), started.recv())
            .await
            .expect("the first pass starts"),
        Some(SyncMode::All)
    );

    online_tx.send(false).expect("session watch is alive");
    tokio::time::timeout(Duration::from_secs(1), async {
        while dropped.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("offline drops the live pass");

    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("sync paused event"),
        Some(Event::SyncPaused)
    ));

    // An automatic incremental request while parked must not downgrade the
    // user's deep pass.
    assert!(!queue.request(SyncMode::Incremental));
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(started.try_recv().is_err(), "no pass runs while offline");

    online_tx.send(true).expect("session watch is alive");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), started.recv())
            .await
            .expect("the pass resumes"),
        Some(SyncMode::All)
    );

    finish.notify_one();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("sync completed event"),
        Some(Event::LibrarySynced)
    ));
    tokio::time::timeout(Duration::from_secs(1), driver)
        .await
        .expect("driver drains")
        .expect("driver task");
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_pass_requested_offline_waits_for_the_session_to_return() {
    let queue = Arc::new(SyncQueue::new());
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let (online_tx, online_rx) = watch::channel(false);
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let finish = Arc::new(Notify::new());

    let run_started = started_tx.clone();
    let run_finish = Arc::clone(&finish);
    let driver = tokio::spawn(drive_syncs(
        Arc::clone(&queue),
        events_tx,
        online_rx,
        move |mode| {
            let started = run_started.clone();
            let finish = Arc::clone(&run_finish);
            async move {
                started.send(mode).expect("run start channel");
                finish.notified().await;
                Ok(())
            }
        },
    ));

    assert!(queue.request(SyncMode::Incremental));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("sync paused event"),
        Some(Event::SyncPaused)
    ));
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        started.try_recv().is_err(),
        "the pass is parked while offline"
    );

    online_tx.send(true).expect("session watch is alive");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), started.recv())
            .await
            .expect("the parked pass starts"),
        Some(SyncMode::Incremental)
    );

    finish.notify_one();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("sync completed event"),
        Some(Event::LibrarySynced)
    ));
    tokio::time::timeout(Duration::from_secs(1), driver)
        .await
        .expect("driver drains")
        .expect("driver task");
}

#[tokio::test]
async fn a_failed_pass_releases_the_slot_for_a_later_request() {
    let queue = Arc::new(SyncQueue::new());
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let (_online_tx, online_rx) = watch::channel(true);

    let driver = tokio::spawn(drive_syncs(
        Arc::clone(&queue),
        events_tx,
        online_rx,
        |_mode| async { Err(anyhow::anyhow!("fixture failure")) },
    ));

    assert!(queue.request(SyncMode::Incremental));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("sync error event"),
        Some(Event::Error(message)) if message == "fixture failure"
    ));
    tokio::time::timeout(Duration::from_secs(1), driver)
        .await
        .expect("driver exits after the failure")
        .expect("driver task");

    // A terminal error must hand the slot back; otherwise a later manual or
    // ping-triggered sync could never start.
    assert!(queue.request(SyncMode::All));
    assert_eq!(queue.next(), Some((SyncMode::All, 1)));
    assert_eq!(queue.next(), None);
}

#[tokio::test]
async fn clearing_a_paused_pass_stops_it_from_resuming() {
    let queue = Arc::new(SyncQueue::new());
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let (online_tx, online_rx) = watch::channel(true);
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let finish = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicUsize::new(0));

    let run_started = started_tx.clone();
    let run_finish = Arc::clone(&finish);
    let run_dropped = Arc::clone(&dropped);
    let driver = tokio::spawn(drive_syncs(
        Arc::clone(&queue),
        events_tx,
        online_rx,
        move |mode| {
            let started = run_started.clone();
            let finish = Arc::clone(&run_finish);
            let dropped = Arc::clone(&run_dropped);
            async move {
                started.send(mode).expect("run start channel");
                let guard = DropCounter(dropped);
                finish.notified().await;
                drop(guard);
                Ok(())
            }
        },
    ));

    assert!(queue.request(SyncMode::Repair));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), started.recv())
            .await
            .expect("the first pass starts"),
        Some(SyncMode::Repair)
    );

    online_tx.send(false).expect("session watch is alive");
    tokio::time::timeout(Duration::from_secs(1), async {
        while dropped.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("offline drops the live pass");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("sync paused event"),
        Some(Event::SyncPaused)
    ));

    // Logout/account switch: the parked pass is no longer valid.
    queue.clear();
    online_tx.send(true).expect("session watch is alive");
    tokio::time::timeout(Duration::from_secs(1), driver)
        .await
        .expect("driver sees the cleared queue and stops")
        .expect("driver task");
    assert!(started.try_recv().is_err(), "the cleared pass does not run");
}
