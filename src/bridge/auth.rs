//! Login and offline-login requests.

use super::*;

impl Bridge {
    pub fn login(&self, server_url: String, username: String, password: String) {
        // A pass from the previous session must not wake up for this account.
        self.sync.clear();
        let tx = self.tx.clone();
        let playback = self.playback.get().cloned();
        crate::app::runtime().spawn(async move {
            flush_status(&playback).await;
            let event = match crate::auth::login(&server_url, &username, &password).await {
                Ok(account) => {
                    crate::session::set_online(true);
                    crate::debug_log!(
                        "bridge: logged in as {} @ {} ({})",
                        account.username,
                        account.server_url,
                        account.server_type
                    );
                    Event::LoggedIn {
                        server_url: account.server_url,
                        username: account.username,
                        offline: false,
                    }
                }
                Err(err) => Event::AuthError(format!("{err:#}")),
            };
            send_event(&tx, event);
        });
    }

    pub fn offline_login(&self, server_url: String, username: String) {
        self.sync.clear();
        let tx = self.tx.clone();
        let playback = self.playback.get().cloned();
        crate::app::runtime().spawn(async move {
            flush_status(&playback).await;
            match crate::auth::offline(&server_url, &username).await {
                Ok(account) => {
                    crate::session::set_online(false);
                    crate::debug_log!(
                        "bridge: offline login as {} @ {}",
                        account.username,
                        account.server_url
                    );
                    send_event(
                        &tx,
                        Event::LoggedIn {
                            server_url: account.server_url,
                            username: account.username,
                            offline: true,
                        },
                    );
                }
                Err(err) => send_event(&tx, Event::AuthError(format!("{err:#}"))),
            }
        });
    }
}

impl Bridge {
    /// End the live session but keep the stored account and its data.
    pub fn logout(&self) {
        let tx = self.tx.clone();
        self.sync.clear();
        self.set_offline(true);
        let playback = self.playback.get().cloned();
        crate::app::runtime().spawn(async move {
            // Persist the outgoing session before the login screen replaces it.
            flush_status(&playback).await;
            crate::session::invalidate().await;
            crate::session::set_online(false);
            _ = tx.send(Event::LoggedOut);
        });
    }

    /// Remove the active account. Data is kept only when `keep_data` is set.
    pub fn remove_account(&self, keep_data: bool) {
        let tx = self.tx.clone();
        self.sync.clear();
        self.set_offline(true);
        let playback = self.playback.get().cloned();
        crate::app::runtime().spawn(async move {
            // The outgoing account's status row must be complete before its
            // pools are closed and its data root deleted (D-07/S-09).
            flush_status(&playback).await;
            crate::session::invalidate().await;
            crate::session::set_online(false);
            let removed_label = crate::accounts::active_account()
                .map(|account| account.label())
                .unwrap_or_else(|| "current account".to_string());
            match crate::accounts::remove_active(keep_data).await {
                Ok(next) => {
                    let next_label = if let Some(account) = next {
                        if let Err(err) = crate::auth::switch_storage(&account).await {
                            _ = tx.send(Event::Error(format!(
                                "Removed {removed_label}, but could not open {}: {err:#}",
                                account.label()
                            )));
                            return;
                        }
                        Some(account.label())
                    } else {
                        if let Err(err) = crate::auth::open_empty_root().await {
                            _ = tx.send(Event::Error(format!(
                                "Removed {removed_label}, but could not open the login database: {err:#}"
                            )));
                            return;
                        }
                        None
                    };
                    _ = tx.send(Event::AccountRemoved {
                        label: removed_label,
                        next: next_label,
                    });
                }
                Err(err) => {
                    _ = tx.send(Event::Error(format!(
                        "Could not remove {removed_label}: {err:#}"
                    )));
                }
            }
        });
    }
}

/// Flushes the live session status when a playback handle is attached.
async fn flush_status(playback: &Option<crate::daemon::playback::PlaybackControl>) {
    if let Some(playback) = playback {
        crate::daemon::flush_status_for_account_switch(playback).await;
    }
}
