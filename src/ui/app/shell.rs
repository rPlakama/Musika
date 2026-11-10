//! Prompt status, toasts and retry actions.

use super::super::*;

/// Label for the held-key bubble: the repeat count plus the distance (or the
/// level) the key accumulates. `count == 1` drops the counter so a single tap
/// reads like a plain announcement.
pub(in crate::ui) fn hold_message(key: char, count: u32, volume: f64) -> String {
    let count = count.max(1);
    let distance = f64::from(count) * HOLD_STEP;
    let (label, detail) = match key {
        ',' => ("Seek back", format!("-{} s", format_seconds(distance))),
        '.' => ("Seek forward", format!("+{} s", format_seconds(distance))),
        '-' => (
            "Volume down",
            format!("{}%", format_level(volume - distance)),
        ),
        '+' => (
            "Volume up",
            format!("{}%", format_level(volume + distance)),
        ),
        _ => return String::new(),
    };
    if count == 1 {
        format!("{label} ({detail})")
    } else {
        format!("{label} x{count} ({detail})")
    }
}

/// Whole seconds, without a trailing `.0` (`15 s`, not `15.0 s`).
fn format_seconds(seconds: f64) -> String {
    if (seconds - seconds.round()).abs() < f64::EPSILON {
        format!("{}", seconds.round() as i64)
    } else {
        format!("{seconds:.1}")
    }
}

/// Volume level the core will land on, clamped like `CoreCommand::SetVolume`.
fn format_level(volume: f64) -> i64 {
    volume.clamp(0.0, 100.0).round() as i64
}

impl AppRoot {
    pub(in crate::ui) fn push_toast(
        &mut self,
        message: String,
        kind: ToastKind,
        retry: Option<RetryAction>,
    ) {
        if self.toasts.iter().any(|toast| toast.message == message) {
            return;
        }
        self.toasts.push(ToastState {
            message,
            kind,
            retry,
            created_at: self.clock.now,
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    /// Records one key-down of a held playback key and refreshes the counter
    /// bubble in place. Returns the folded hold.
    pub(in crate::ui) fn hold_playback_key(&mut self, key: char) -> KeyHold {
        let now_ns = crate::linux::monotonic_ns();
        let hold = KeyHold::record(self.playback_key_hold, key, now_ns);
        self.playback_key_hold = Some(hold);
        self.hold_toast = Some(HoldToast::new(
            hold_message(hold.key, hold.count, self.player.volume()),
            now_ns,
        ));
        hold
    }

    pub(in crate::ui) fn retry(&mut self, action: RetryAction) {
        match action {
            RetryAction::ReloadLibrary => self.reload(),
            RetryAction::Playback => {
                if let Some((song, order)) = self.last_play.clone() {
                    self.bridge.play_song(song, order);
                }
            }
        }
    }
}
