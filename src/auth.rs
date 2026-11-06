// Musika — toolkit-free authentication decisions for the GPUI front end.

use anyhow::Result;

pub struct Account {
    pub server_url: String,
    pub username: String,
    pub server_type: String,
}

pub async fn login(server_url: &str, username: &str, password: &str) -> Result<Account> {
    let info = crate::http_client::authenticate(server_url, username, password).await?;
    crate::session::install(server_url, username, password).await;
    if let Some(dbs) = crate::app::dbs()
        && let Err(err) = crate::sqlite::save_credentials(
            &dbs.credentials,
            username,
            password,
            server_url,
            &info.kind,
        )
        .await
    {
        eprintln!("auth: could not persist credentials: {err:#}");
    }
    crate::session::set_online(true);
    Ok(Account {
        server_url: server_url.to_string(),
        username: username.to_string(),
        server_type: info.kind,
    })
}

pub async fn offline(server_url: &str, username: &str) -> Account {
    crate::session::set_online(false);
    Account {
        server_url: server_url.to_string(),
        username: username.to_string(),
        server_type: "offline".to_string(),
    }
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
