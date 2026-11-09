//! Frame counters for `MUSIKA_FRAME_STATS=1`.
//!
//! These are process-local diagnostics: no work is done unless the environment
//! variable is set at startup. The counters are cheap enough to leave in a
//! release build (one `Instant::now()` per frame plus a ring push).

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

use super::AppRoot;

/// Samples kept for the percentile window.
const WINDOW: usize = 600;
/// Frames between two summary lines.
const REPORT_EVERY: usize = 120;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MUSIKA_FRAME_STATS").is_some())
}

struct Stats {
    intervals: [f32; WINDOW],
    render_ms: [f32; WINDOW],
    head: usize,
    len: usize,
    since_report: usize,
    last_shapes: u64,
    last_raf: std::collections::HashMap<&'static str, u64>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            intervals: [0.0; WINDOW],
            render_ms: [0.0; WINDOW],
            head: 0,
            len: 0,
            since_report: 0,
            last_shapes: 0,
            last_raf: std::collections::HashMap::new(),
        }
    }
}

impl Stats {
    fn push(&mut self, interval_ms: f32, render_ms: f32) {
        self.intervals[self.head] = interval_ms;
        self.render_ms[self.head] = render_ms;
        self.head = (self.head + 1) % WINDOW;
        self.len = (self.len + 1).min(WINDOW);
        self.since_report += 1;
    }
}

thread_local! {
    static STATS: RefCell<Stats> = RefCell::new(Stats::default());
}

/// Ordered copy of the ring window (oldest first).
fn window(values: &[f32; WINDOW], head: usize, len: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(len);
    for index in 0..len {
        out.push(values[(head + WINDOW - len + index) % WINDOW]);
    }
    out
}

/// Nearest-rank percentile of a sorted slice.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f32 * p.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

/// Resident set size in MiB, read from `/proc/self/status` (Linux).
fn rss_mib() -> f32 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0.0;
    };
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        let kb = rest
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0);
        return kb / 1024.0;
    }
    0.0
}

/// Records one completed `render()` and prints a summary every
/// [`REPORT_EVERY`] frames. Call only when the main window rendered (the login
/// screen is not measured).
pub(crate) fn record_frame(started: Instant, root: &AppRoot) {
    let render_ms = started.elapsed().as_secs_f32() * 1000.0;
    let interval_ms = root.clock.dt * 1000.0;
    let shapes = super::widgets::retained_text::shape_count();
    let report = STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.push(interval_ms, render_ms);
        if !enabled() || stats.since_report < REPORT_EVERY {
            return None;
        }
        let since_report = stats.since_report;
        stats.since_report = 0;
        let intervals = window(&stats.intervals, stats.head, stats.len);
        let render = window(&stats.render_ms, stats.head, stats.len);
        let shapes_delta = shapes.saturating_sub(stats.last_shapes);
        stats.last_shapes = shapes;
        let mut intervals_sorted = intervals.clone();
        intervals_sorted.sort_by(f32::total_cmp);
        let mut render_sorted = render.clone();
        render_sorted.sort_by(f32::total_cmp);
        let raf_now = RAF_SITES.with(|sites| sites.borrow().clone());
        let mut raf_delta: Vec<(&'static str, u64)> = raf_now
            .iter()
            .map(|(site, count)| {
                (*site, count.saturating_sub(*stats.last_raf.get(site).unwrap_or(&0)))
            })
            .filter(|(_, delta)| *delta > 0)
            .collect();
        raf_delta.sort_by(|a, b| b.1.cmp(&a.1));
        stats.last_raf = raf_now;
        Some((
            since_report,
            mean(&intervals),
            percentile(&intervals_sorted, 0.95),
            mean(&render),
            percentile(&render_sorted, 0.95),
            shapes_delta,
            raf_delta,
        ))
    });
    let Some((
        frames,
        mean_interval,
        p95_interval,
        mean_render,
        p95_render,
        shapes_delta,
        raf_delta,
    )) = report
    else {
        return;
    };
    let fps = if mean_interval > 0.0 {
        1000.0 / mean_interval
    } else {
        0.0
    };
    let rows = root.view.rows.live.len();
    let page_rows = root.page.as_ref().map_or(0, |page| page.rows.len());
    let scales = root.row_scales.borrow().len();
    let toasts = root.toasts.len();
    eprintln!(
        "[musika/frame] frames={frames} fps={fps:.1} interval_ms(mean={mean_interval:.2} p95={p95_interval:.2}) render_ms(mean={mean_render:.2} p95={p95_render:.2}) rows={rows} live_start={} count={} page_rows={page_rows} scales={scales} toasts={toasts} shapes+={shapes_delta} win={:.0}x{:.0} state={:?} loading={} syncing={} queue={} fullscreen={} active={} rss={:.0}MiB",
        root.view.rows.live_start,
        root.view.rows.count,
        root.win_w,
        root.win_h,
        root.player.state(),
        root.player.is_loading(),
        root.syncing,
        root.queue_open,
        root.fullscreen,
        super::perf::window_active() as u8,
        rss_mib(),
    );
    let raf: Vec<String> = raf_delta
        .iter()
        .map(|(site, count)| format!("{site}:{count}"))
        .collect();
    if !raf.is_empty() {
        eprintln!("[musika/raf] {}", raf.join(" "));
    }
}

// ---------------------------------------------------------------------------
// Animation-frame call sites. Every `request_animation_frame` in the UI goes
// through here so `MUSIKA_FRAME_STATS` can attribute a runaway repaint chain to
// the site that caused it.
// ---------------------------------------------------------------------------

thread_local! {
    static RAF_SITES: RefCell<std::collections::HashMap<&'static str, u64>> =
        RefCell::new(std::collections::HashMap::new());
}

#[cfg(test)]
pub(crate) fn reset_raf_sites() {
    RAF_SITES.with(|sites| sites.borrow_mut().clear());
}

/// Requests one frame, attributing it to `site` for the frame summary.
pub(crate) fn request_frame(window: &mut gpui::Window, site: &'static str) {
    if enabled() {
        RAF_SITES.with(|sites| {
            *sites.borrow_mut().entry(site).or_insert(0) += 1;
        });
    }
    window.request_animation_frame();
}

// ---------------------------------------------------------------------------
// Frame pacing: the app requests repaints from one ticker instead of asking
// for a frame on every render (the GPUI request itself is a full-tree redraw).
// ---------------------------------------------------------------------------

/// Sources that can keep the UI moving. Built by the ticker from live state.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Demand {
    /// Transport is playing: the progress bar and time label move.
    pub playing: bool,
    /// The window is not focused: only slow progress upkeep is needed.
    pub inactive: bool,
    /// Next time a marquee needs a frame (perf clock, seconds), if any.
    pub marquee_at: Option<f64>,
}

/// One ticker decision: whether to notify the root view now, and how long to
/// sleep before the next check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    pub notify: bool,
    pub sleep_ms: u64,
}

/// Full rate while playing (~10 Hz is sub-pixel on a normal progress bar).
const PLAYING_INTERVAL: f64 = 0.1;
/// Inactive windows only need the clock to keep moving.
const INACTIVE_INTERVAL: f64 = 1.0;
/// Marquee travel advances at most 30 times a second.
const MARQUEE_INTERVAL: f64 = 1.0 / 30.0;
/// Never sleep longer than this, so input/events are picked up promptly.
const MAX_SLEEP_MS: u64 = 250;

/// Pure pacing decision. `last_notify` is the last render/notify timestamp on
/// the same clock as `now` (seconds).
pub(crate) fn decide(now: f64, demand: Demand, last_notify: f64) -> Decision {
    // The shortest active source wins.
    let mut interval = if demand.playing {
        Some(if demand.inactive {
            INACTIVE_INTERVAL
        } else {
            PLAYING_INTERVAL
        })
    } else {
        None
    };
    if demand.marquee_at.is_some() && !demand.inactive {
        interval = Some(interval.map_or(MARQUEE_INTERVAL, |current| current.min(MARQUEE_INTERVAL)));
    }

    let mut sleep_ms = match interval {
        Some(interval) => ((interval * 1000.0).ceil() as u64).clamp(5, MAX_SLEEP_MS),
        None => MAX_SLEEP_MS,
    };
    // Tolerate float noise in accumulated timestamps (e.g. 10.1 - 10.0).
    const EPSILON: f64 = 1e-9;
    let mut notify = interval.is_some_and(|interval| now - last_notify + EPSILON >= interval);

    // A marquee wake is exact: sleep until the next travel/pause boundary
    // instead of spinning at 30 Hz through a static hold.
    if let Some(at) = demand.marquee_at {
        if at <= now {
            notify = true;
            sleep_ms = sleep_ms.min(5);
        } else if !demand.inactive {
            let wake = ((at - now) * 1000.0).ceil() as u64;
            sleep_ms = sleep_ms.min(wake.max(5));
        }
    }
    Decision { notify, sleep_ms }
}

/// Monotonic clock shared by the ticker and the marquee elements.
pub(crate) fn now() -> f64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

thread_local! {
    static LAST_NOTIFY: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::NEG_INFINITY) };
    static LAST_PUMP: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::NEG_INFINITY) };
    static MARQUEE_AT: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::INFINITY) };
}

/// Publishes when a marquee next needs a frame (`now + delay`, or now while it
/// is travelling). Multiple marquees keep the earliest deadline.
pub(crate) fn note_marquee(delay: f64) {
    let at = now() + delay.max(1.0 / 60.0);
    MARQUEE_AT.with(|slot| {
        let current = slot.get();
        if current <= now() || at < current {
            slot.set(at);
        }
    });
}

/// Earliest marquee deadline, if any.
pub(crate) fn marquee_deadline() -> Option<f64> {
    let at = MARQUEE_AT.with(|slot| slot.get());
    at.is_finite().then_some(at)
}

/// Returns true when a marquee wake is due and re-arms it for the next paint.
pub(crate) fn take_marquee_due(now: f64) -> bool {
    MARQUEE_AT.with(|slot| {
        if slot.get() <= now {
            slot.set(f64::INFINITY);
            true
        } else {
            false
        }
    })
}

pub(crate) fn last_notify() -> f64 {
    LAST_NOTIFY.with(|slot| slot.get())
}

/// Records that a render (or a ticker notify) happened at `now`.
pub(crate) fn note_notify(now: f64) {
    LAST_NOTIFY.with(|slot| slot.set(now));
}

/// At most one pump per 100 ms, independent of render cadence.
pub(crate) fn pump_due(now: f64) -> bool {
    if now - LAST_PUMP.with(|slot| slot.get()) < PLAYING_INTERVAL {
        return false;
    }
    LAST_PUMP.with(|slot| slot.set(now));
    true
}

// ---------------------------------------------------------------------------
// Frame pacing state (used by the repaint ticker / elements).
// ---------------------------------------------------------------------------

thread_local! {
    static WINDOW_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}
pub(crate) fn set_window_active(active: bool) {
    WINDOW_ACTIVE.with(|slot| slot.set(active));
}

pub(crate) fn window_active() -> bool {
    WINDOW_ACTIVE.with(|slot| slot.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_nearest_rank() {
        let mut values = [1.0, 2.0, 3.0, 4.0, 5.0];
        values.sort_by(f32::total_cmp);
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&values, 0.5), 3.0);
        assert_eq!(percentile(&values, 1.0), 5.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn playing_repaints_at_ten_hz_and_idles_otherwise() {
        let playing = Demand {
            playing: true,
            ..Demand::default()
        };
        // First frame after playback started: notify immediately.
        let first = decide(10.0, playing, f64::NEG_INFINITY);
        assert!(first.notify);
        assert_eq!(first.sleep_ms, 100);
        // 50 ms later: not due yet.
        let early = decide(10.05, playing, 10.0);
        assert!(!early.notify);
        // 100 ms later: due.
        let due = decide(10.1, playing, 10.0);
        assert!(due.notify);
        // Stopped: no notifications, but keep waking up.
        let idle = decide(20.0, Demand::default(), 10.2);
        assert!(!idle.notify);
        assert_eq!(idle.sleep_ms, MAX_SLEEP_MS);
    }

    #[test]
    fn inactive_window_falls_back_to_one_hz() {
        let inactive = Demand {
            playing: true,
            inactive: true,
            ..Demand::default()
        };
        let first = decide(1.0, inactive, 0.9);
        assert!(!first.notify);
        let due = decide(1.0, inactive, 0.0);
        assert!(due.notify);
        assert_eq!(due.sleep_ms, MAX_SLEEP_MS);
    }

    #[test]
    fn marquee_waites_for_its_deadline_instead_of_spinning() {
        let travel = Demand {
            marquee_at: Some(10.02),
            ..Demand::default()
        };
        let decision = decide(10.0, travel, 9.99);
        assert!(!decision.notify);
        assert_eq!(decision.sleep_ms, 20);
        let due = decide(10.02, travel, 9.99);
        assert!(due.notify);
        assert_eq!(due.sleep_ms, 5);
    }

    #[test]
    fn playing_and_marquee_use_the_faster_cadence() {
        let both = Demand {
            playing: true,
            marquee_at: Some(100.0),
            ..Demand::default()
        };
        let decision = decide(10.0, both, 9.99);
        // Marquee travel wants 33 ms even while the transport only needs 100.
        assert_eq!(decision.sleep_ms, 34);
    }

    #[test]
    fn window_is_oldest_first_and_wraps() {
        let mut values = [0.0; WINDOW];
        // Simulate head=3 after exactly WINDOW pushes.
        values[0] = 600.0;
        values[1] = 601.0;
        values[2] = 602.0;
        let out = window(&values, 3, 3);
        assert_eq!(out, vec![600.0, 601.0, 602.0]);
        let out = window(&values, 0, 3);
        assert_eq!(out, vec![0.0, 0.0, 0.0]);
    }
}
