//! Bridge request family.

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
            let account = crate::auth::offline(&server_url, &username).await;
            crate::debug_log!("GPUI: offline login as {} @ {}", account.username, account.server_url);
            send_event(
                &tx,
                Event::LoggedIn {
                    server_url: account.server_url,
                    username: account.username,
                    offline: true,
                },
            );
        });
    }
}
