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
                    let now = perf::now();
                    // The transport advances without a render: pump on a fixed
                    // cadence, and surface a playback error immediately.
                    if this.player.state() != crate::player::State::Stopped
                        && perf::pump_due(now)
                        && let Some(message) = this.player.pump()
                    {
                        this.playback_error = Some(message);
                        cx.notify();
                    }
                    let marquee_due = perf::take_marquee_due(now);
                    let demand = this.frame_demand();
                    let changed = perf::demand_changed(&demand);
                    let decision = perf::decide(now, demand, perf::last_notify());
                    if decision.notify || marquee_due || changed {
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

    /// What can move right now.
    fn frame_demand(&self) -> perf::Demand {
        let inactive = !perf::window_active();
        // Toasts use the animation clock for `created_at`; compare on the same
        // origin. The fade starts at age 6.0 s and the toast is removed at 6.32.
        let now = self.clock.peek();
        let mut fade_in: Option<f64> = None;
        if !inactive {
            for toast in &self.toasts {
                let age = now - toast.created_at;
                if age < 6.0 {
                    let remaining = 6.0 - age;
                    fade_in = Some(fade_in.map_or(remaining, |current: f64| current.min(remaining)));
                }
            }
        }
        // Legacy runner.qml `idleBehavior = !active && !nowPlayingVisible`:
        // the expanded player (fullscreen / lyrics overlay) must keep moving
        // even when another compositor window has focus.
        let now_playing = self.fullscreen || self.lyrics_overlay;
        perf::Demand {
            playing: self.player.state() == crate::player::State::Playing,
            inactive,
            now_playing,
            loading: self.player.is_loading(),
            syncing: self.syncing,
            toast_active: !self.toasts.is_empty(),
            toast_fade_in: fade_in,
            marquee_at: perf::marquee_deadline(),
        }
    }
}
