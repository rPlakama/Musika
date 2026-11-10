//! Prompt status, toasts and retry actions.

use super::super::*;

/// Chevron shown next to a volume bubble. `up-fill`/`down-fill` are the icon
/// set's own arrows, so the sign reads at a glance.
const ICON_UP: &str = "up-fill.svg";
const ICON_DOWN: &str = "down-fill.svg";

/// Label for the held-key bubble: the repeat count plus the signed distance (or
/// the level) the key group accumulated. `count == 1` drops the counter so a
/// single tap reads like a plain announcement, and a mixed `,.` spam that nets
/// out still reports its count with a `0 s` delta.
pub(in crate::ui) fn hold_message(hold: KeyHold, volume: f64) -> (String, Option<&'static str>) {
    let count = hold.count.max(1);
    let suffix = if count == 1 {
        String::new()
    } else {
        format!(" x{count}")
    };
    match hold.kind {
        HoldKind::Seek => {
            // A mixed `,.` burst can net out; say so instead of claiming a
            // direction the playhead never moved in.
            if hold.delta == 0.0 {
                return (format!("Seek{suffix} (0 s)"), None);
            }
            let label = if hold.delta < 0.0 {
                "Seek back"
            } else {
                "Seek forward"
            };
            let detail = format!("{} s", format_seconds(hold.delta));
            (format!("{label}{suffix} ({detail})"), None)
        }
        HoldKind::Volume => {
            let level = format_level(volume + hold.delta);
            if hold.delta == 0.0 {
                return (format!("Volume{suffix} ({level}%)"), None);
            }
            let icon = if hold.delta < 0.0 { ICON_DOWN } else { ICON_UP };
            let label = if hold.delta < 0.0 {
                "Volume down"
            } else {
                "Volume up"
            };
            (format!("{label}{suffix} ({level}%)"), Some(icon))
        }
    }
}

/// Signed seconds without a trailing `.0` (`+15`, `-5`, `0`).
fn format_seconds(seconds: f64) -> String {
    if (seconds - seconds.round()).abs() < f64::EPSILON {
        format!("{:+}", seconds.round() as i64)
    } else {
        format!("{seconds:+.1}")
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
        let (message, icon) = hold_message(hold, self.player.volume());
        self.hold_toast = Some(HoldToast::new(message, icon, now_ns));
        hold
    }

    /// Sticky mute bubble: it has no TTL and is only cleared once the mute state
    /// is off, so it stays on screen until unmuted. Callers pass the state the
    /// core reported, or the expected one right after a local toggle.
    pub(in crate::ui) fn sync_mute_toast(&mut self, muted: bool) {
        if muted {
            // The level that will come back on unmute: mute never changes it.
            let level = self
                .mute_toast_level
                .unwrap_or_else(|| format_level(self.player.volume()));
            self.mute_toast_level = Some(level);
            self.mute_toast = Some(MuteToast {
                message: format!("Muted ({level}%)"),
                created_at: self
                    .mute_toast
                    .as_ref()
                    .map_or(self.clock.now, |toast| toast.created_at),
            });
        } else {
            self.mute_toast = None;
            self.mute_toast_level = None;
        }
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
