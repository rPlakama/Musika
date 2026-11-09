//! Login and offline-login requests.

use super::*;

impl Bridge {
    pub fn login(&self, server_url: String, username: String, password: String) {
        self.set_offline(false);
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match crate::auth::login(&server_url, &username, &password).await {
                Ok(account) => {
                    crate::debug_log!(
                        "GPUI: logged in as {} @ {} ({})",
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
        self.set_offline(true);
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            match crate::auth::offline(&server_url, &username).await {
                Ok(account) => {
                    crate::debug_log!(
                        "GPUI: offline login as {} @ {}",
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
        self.set_offline(true);
        crate::app::runtime().spawn(async move {
            crate::session::invalidate().await;
            crate::session::set_online(false);
            let _ = tx.send(Event::LoggedOut);
        });
    }

    /// Remove the active account. Data is kept only when `keep_data` is set.
    pub fn remove_account(&self, keep_data: bool) {
        let tx = self.tx.clone();
        self.set_offline(true);
        crate::app::runtime().spawn(async move {
            crate::session::invalidate().await;
            crate::session::set_online(false);
            let removed_label = crate::accounts::active_account()
                .map(|account| account.label())
                .unwrap_or_else(|| "current account".to_string());
            match crate::accounts::remove_active(keep_data) {
                Ok(next) => {
                    let next_label = if let Some(account) = next {
                        if let Err(err) = crate::auth::switch_storage(&account).await {
                            let _ = tx.send(Event::Error(format!(
                                "Removed {removed_label}, but could not open {}: {err:#}",
                                account.label()
                            )));
                            return;
                        }
                        Some(account.label())
                    } else {
                        if let Err(err) = crate::auth::open_empty_root().await {
                            let _ = tx.send(Event::Error(format!(
                                "Removed {removed_label}, but could not open the login database: {err:#}"
                            )));
                            return;
                        }
                        None
                    };
                    let _ = tx.send(Event::AccountRemoved {
                        label: removed_label,
                        next: next_label,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!(
                        "Could not remove {removed_label}: {err:#}"
                    )));
                }
            }
        });
    }
}
