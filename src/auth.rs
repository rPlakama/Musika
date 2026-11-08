// Musika — toolkit-free authentication decisions for the GPUI front end.

use anyhow::Result;

pub struct Account {
    pub server_url: String,
    pub username: String,
    pub server_type: String,
}

pub async fn login(server_url: &str, username: &str, password: &str) -> Result<Account> {
    let info = crate::http_client::authenticate(server_url, username, password).await?;
    let stored = crate::accounts::upsert_and_activate(server_url, username, password, &info.kind)?;
    switch_storage(&stored).await?;
    crate::session::install(server_url, username, password).await;
    crate::session::set_online(true);
    Ok(Account {
        server_url: stored.server_url,
        username: stored.username,
        server_type: info.kind,
    })
}

pub async fn offline(server_url: &str, username: &str) -> Result<Account> {
    let stored = crate::accounts::activate_stored(server_url, username)?;
    switch_storage(&stored).await?;
    crate::session::invalidate().await;
    crate::session::set_online(false);
    Ok(Account {
        server_url: stored.server_url,
        username: stored.username,
        server_type: stored.server_type,
    })
}

/// Opens the stored account's data root and swaps the process-wide pools.
pub(crate) async fn switch_storage(account: &crate::accounts::Account) -> Result<()> {
    crate::accounts::ensure_data_root(account)?;
    if crate::db::data_root()? == account.data_root {
        return Ok(());
    }
    crate::db::set_data_root(account.data_root.clone());
    let dbs = crate::db::init().await?;
    crate::app::set_dbs(dbs);
    crate::session::invalidate().await;
    Ok(())
}

pub async fn downloaded_available() -> bool {
    match crate::app::dbs() {
        Some(dbs) => crate::download::counts(&dbs.download)
            .await
            .map(|counts| counts.done > 0)
            .unwrap_or(false),
        None => false,
    }
}

/// Opens the legacy/bootstrap root after the last account is removed.
pub(crate) async fn open_empty_root() -> Result<()> {
    let root = crate::db::config_dir()?;
    if crate::db::data_root()? == root {
        return Ok(());
    }
    crate::db::set_data_root(root);
    let dbs = crate::db::init().await?;
    crate::app::set_dbs(dbs);
    crate::session::invalidate().await;
    Ok(())
}
