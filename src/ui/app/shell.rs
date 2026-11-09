//! Prompt status, toasts and retry actions.

use super::super::*;

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
