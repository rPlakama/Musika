//! The single repaint ticker.
//!
//! GPUI's `request_animation_frame` re-renders the whole root view, so asking
//! for one on every playing frame (the old behaviour) rebuilt the entire tree
//! at the display refresh for a 3 px progress bar. This ticker is the one place
//! that decides when a playback-driven frame is due, and it also pumps the
//! native player so MPRIS/position stay current without rendering.

use std::time::Duration;

use gpui::Context;

use super::super::AppRoot;
use super::super::perf;

impl AppRoot {
    /// Spawns the repaint ticker (one per app). It wakes at most every
    /// [`perf::MAX_SLEEP_MS`] and notifies the root only when a source is due,
    /// so an idle or paused window costs one cheap check per wakeup.
    pub(in crate::ui) fn spawn_frame_ticker(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let decision = this.update(cx, |this, cx| {
                    let now = this.clock.peek();
                    // The transport advances without a render: pump on a fixed
                    // cadence, and surface a playback error immediately.
                    if this.player.state() != crate::player::State::Stopped
                        && perf::pump_due(now)
                        && let Some(message) = this.player.pump()
                    {
                        this.playback_error = Some(message);
                        cx.notify();
                    }
                    let demand = this.frame_demand();
                    let decision = perf::decide(now, demand, perf::last_notify());
                    if decision.notify {
                        perf::note_notify(now);
                        cx.notify();
                    }
                    decision
                });
                let Ok(decision) = decision else {
                    break;
                };
                cx.background_executor()
                    .timer(Duration::from_millis(decision.sleep_ms))
                    .await;
            }
        })
        .detach();
    }

    /// What can move right now. Extended by the toast/marquee/spinner tasks.
    fn frame_demand(&self) -> perf::Demand {
        perf::Demand {
            playing: self.player.state() == crate::player::State::Playing,
            inactive: !perf::window_active(),
        }
    }
}
