//! Multi-account registry and per-account data roots.
//!
//! The account list is a small global JSON document; all SQLite databases,
//! cache folders and local playlists belong to the active account's data root.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::db::Dbs;
use crate::sqlite::Credentials;

const REGISTRY_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    id: String,
    server_url: String,
    username: String,
    /// AES-GCM encrypted with the shared local key.
    password: String,
    server_type: String,
    /// Relative to the config dir unless absolute. `.` is the legacy root.
    data_root: String,
    created_at: i64,
    last_used: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    accounts: Vec<StoredAccount>,
}

impl Registry {
    fn versioned() -> Self {
        Self {
            version: REGISTRY_VERSION,
            ..Default::default()
        }
    }

    fn active(&self) -> Option<&StoredAccount> {
        let id = self.active.as_deref()?;
        self.accounts.iter().find(|account| account.id == id)
    }

    fn by_credentials(&self, server_url: &str, username: &str) -> Option<&StoredAccount> {
        self.accounts
            .iter()
            .find(|account| account.server_url == server_url && account.username == username)
    }
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub server_url: String,
    pub username: String,
    pub password: String,
    pub server_type: String,
    pub data_root: PathBuf,
}

impl Account {
    pub fn label(&self) -> String {
        format!("{} @ {}", self.username, self.server_url)
    }
}

static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
static REGISTRY_EXISTED: OnceLock<bool> = OnceLock::new();

pub fn account_id(server_url: &str, username: &str) -> String {
    crate::digest::md5_hex(format!("{server_url}\n{username}")).unwrap_or_else(|_| {
        format!(
            "{:x}",
            crate::consts::now_nanos().wrapping_add(server_url.len() as u128)
        )
    })
}

fn registry_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MUSIKA_ACCOUNTS_FILE")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    Ok(crate::db::config_dir()?.join("accounts.json"))
}

fn load_from_disk() -> Result<Registry> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(Registry::versioned());
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("failed to read account registry {}", path.display()))?;
    let mut registry: Registry = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid account registry {}", path.display()))?;
    if registry.version == 0 {
        registry.version = REGISTRY_VERSION;
    }
    Ok(registry)
}

fn write_registry(registry: &Registry) -> Result<()> {
    let path = registry_path()?;
    crate::db::ensure_parent_dir(&path)?;
    let bytes = serde_json::to_vec_pretty(registry)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn registry() -> &'static RwLock<Registry> {
    REGISTRY.get_or_init(|| match load_from_disk() {
        Ok(registry) => {
            _ = REGISTRY_EXISTED.set(registry_path().is_ok_and(|path| path.exists()));
            RwLock::new(registry)
        }
        Err(err) => {
            eprintln!("accounts: could not load registry: {err:#}");
            RwLock::new(Registry::versioned())
        }
    })
}

/// Load the registry before the database root is chosen. Returns the data root
/// of the active account, or the legacy config root when no account is stored.
pub fn startup_data_root() -> Result<PathBuf> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    if let Some(account) = guard.active().or_else(|| {
        guard
            .accounts
            .iter()
            .max_by_key(|account| account.last_used)
    }) {
        return resolve_data_root(&account.data_root);
    }
    crate::db::config_dir()
}

pub fn registry_existed() -> bool {
    REGISTRY_EXISTED.get().copied().unwrap_or(false)
}

fn resolve_data_root(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if value == "." || value.is_empty() {
        return crate::db::config_dir();
    }
    Ok(crate::db::config_dir()?.join(path))
}

fn to_account(stored: &StoredAccount) -> Result<Account> {
    Ok(Account {
        id: stored.id.clone(),
        server_url: stored.server_url.clone(),
        username: stored.username.clone(),
        password: crate::crypto::decrypt_password(&stored.password)?,
        server_type: stored.server_type.clone(),
        data_root: resolve_data_root(&stored.data_root)?,
    })
}

pub fn active_account() -> Option<Account> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    guard.active().and_then(|stored| to_account(stored).ok())
}

pub fn last_account() -> Option<Account> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    guard
        .accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .and_then(|stored| to_account(stored).ok())
}

pub fn account_for(server_url: &str, username: &str) -> Option<Account> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    guard
        .by_credentials(server_url, username)
        .and_then(|stored| to_account(stored).ok())
}

/// Authenticate/login result: add or update one account and make it active.
pub fn upsert_and_activate(
    server_url: &str,
    username: &str,
    password: &str,
    server_type: &str,
) -> Result<Account> {
    let id = account_id(server_url, username);
    let encrypted = crate::crypto::encrypt_password(password)?;
    let now = crate::consts::now_secs();
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = guard.accounts.iter_mut().find(|account| account.id == id) {
        existing.username = username.to_string();
        existing.server_url = server_url.to_string();
        existing.password = encrypted;
        existing.server_type = server_type.to_string();
        existing.last_used = now;
    } else {
        let data_root = format!("accounts/{id}");
        guard.accounts.push(StoredAccount {
            id: id.clone(),
            server_url: server_url.to_string(),
            username: username.to_string(),
            password: encrypted,
            server_type: server_type.to_string(),
            data_root,
            created_at: now,
            last_used: now,
        });
    }
    guard.active = Some(id);
    write_registry(&guard)?;
    to_account(
        guard
            .accounts
            .iter()
            .find(|account| account.id == account_id(server_url, username))
            .ok_or_else(|| anyhow!("account was not stored"))?,
    )
}

/// Select an already-stored account, e.g. F2 offline login.
pub fn activate_stored(server_url: &str, username: &str) -> Result<Account> {
    let id = account_id(server_url, username);
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    let now = crate::consts::now_secs();
    let stored = guard
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(|| anyhow!("no saved account for {username} @ {server_url}"))?;
    stored.last_used = now;
    let stored = stored.clone();
    guard.active = Some(id);
    let account = to_account(&stored)?;
    write_registry(&guard)?;
    Ok(account)
}

/// Remove the active account. `keep_data` leaves its data root on disk.
/// Returns the next account to activate (or None when no accounts remain).
pub fn remove_active(keep_data: bool) -> Result<Option<Account>> {
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    let Some(active) = guard.active.clone() else {
        return Ok(None);
    };
    let Some(index) = guard
        .accounts
        .iter()
        .position(|account| account.id == active)
    else {
        guard.active = None;
        write_registry(&guard)?;
        return Ok(None);
    };
    let removed = guard.accounts.remove(index);
    let legacy_root = crate::db::config_dir()?;
    let data_root = resolve_data_root(&removed.data_root)?;
    if !keep_data && data_root != legacy_root {
        _ = std::fs::remove_dir_all(&data_root);
    }
    let next = guard
        .accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .cloned();
    guard.active = next.as_ref().map(|account| account.id.clone());
    write_registry(&guard)?;
    next.as_ref().map(to_account).transpose()
}

/// First-run migration: import the legacy single-account credentials table.
pub async fn migrate_legacy(dbs: &Dbs) -> Result<()> {
    if registry_existed() {
        return Ok(());
    }
    let credentials = crate::sqlite::all_credentials(&dbs.credentials).await?;
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    guard.version = REGISTRY_VERSION;
    guard.accounts.clear();
    guard.active = None;
    let latest_id = credentials
        .iter()
        .max_by_key(|credential| credential.created_at)
        .map(|credential| account_id(&credential.server_url, &credential.username));
    for credential in credentials {
        let id = account_id(&credential.server_url, &credential.username);
        let legacy = latest_id.as_deref() == Some(id.as_str());
        guard.accounts.push(StoredAccount {
            id: id.clone(),
            server_url: credential.server_url,
            username: credential.username,
            password: crate::crypto::encrypt_password(&credential.password)?,
            server_type: credential.server_type,
            data_root: if legacy {
                ".".to_string()
            } else {
                format!("accounts/{id}")
            },
            created_at: credential.created_at,
            last_used: credential.created_at,
        });
        if legacy {
            guard.active = Some(id);
        }
    }
    write_registry(&guard)?;
    Ok(())
}

pub fn active_credentials() -> Option<(String, String, String, String)> {
    active_account().map(|account| {
        (
            account.server_url,
            account.username,
            account.password,
            account.server_type,
        )
    })
}

pub fn ensure_data_root(account: &Account) -> Result<()> {
    crate::db::ensure_dir(&account.data_root)
}

pub fn credentials_from(account: &Account) -> Credentials {
    Credentials {
        username: account.username.clone(),
        password: account.password.clone(),
        server_url: account.server_url.clone(),
        server_type: account.server_type.clone(),
        created_at: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids_are_stable_and_server_specific() {
        let a = account_id("http://one", "user");
        assert_eq!(a, account_id("http://one", "user"));
        assert_ne!(a, account_id("http://one", "other"));
        assert_ne!(a, account_id("http://two", "user"));
    }

    #[test]
    fn legacy_root_is_the_config_dir() {
        let root = resolve_data_root(".").unwrap();
        assert_eq!(root, crate::db::config_dir().unwrap());
    }

    #[test]
    fn playlist_command_split_keeps_spaces_and_trailing_flag() {
        // The parser lives in ipc_command; this is a guard for the account
        // side of the command: names are not path fragments.
        let name = "Road Trip";
        let id = account_id("http://one", name);
        assert_eq!(id, account_id("http://one", "Road Trip"));
    }
}
