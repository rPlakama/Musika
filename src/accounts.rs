//! Multi-account registry and per-account data roots.
//!
//! The account list is a small global JSON document; all SQLite databases,
//! cache folders and local playlists belong to the active account's data root.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::db::Dbs;

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

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the account-registry tests exercise this lookup"
        )
    )]
    fn by_credentials(&self, server_url: &str, username: &str) -> Option<&StoredAccount> {
        self.accounts
            .iter()
            .find(|account| account.server_url == server_url && account.username == username)
    }
}

#[derive(Debug, Clone)]
pub struct Account {
    pub server_url: String,
    pub username: String,
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "only the non-test session cache reads the decrypted password"
        )
    )]
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
    use std::io::Write;

    let path = registry_path()?;
    crate::db::ensure_parent_dir(&path)?;
    let bytes = serde_json::to_vec_pretty(registry)?;
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("failed to create {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(&bytes)
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn registry() -> &'static RwLock<Registry> {
    REGISTRY.get_or_init(|| {
        let existed = registry_path().map_or(true, |path| path.exists());
        _ = REGISTRY_EXISTED.set(existed);
        match load_from_disk() {
            Ok(registry) => RwLock::new(registry),
            Err(err) => {
                eprintln!("accounts: could not load registry: {err:#}");
                RwLock::new(Registry::versioned())
            }
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
    let _ = registry();
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

/// One account's non-secret identity fields.
///
/// Status flushes and header refreshes only need these; they must not pay the
/// AES-GCM password decrypt (or allocate a password) just to show a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub server_url: String,
    pub username: String,
    pub server_type: String,
}

/// The active account without decrypting or copying its password.
pub fn active_identity() -> Option<Identity> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    guard.active().map(|stored| Identity {
        server_url: stored.server_url.clone(),
        username: stored.username.clone(),
        server_type: stored.server_type.clone(),
    })
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the account-registry tests exercise this lookup"
    )
)]
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
    let mut candidate = guard.clone();
    if let Some(existing) = candidate
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
    {
        existing.username = username.to_string();
        existing.server_url = server_url.to_string();
        existing.password = encrypted;
        existing.server_type = server_type.to_string();
        existing.last_used = now;
    } else {
        let data_root = format!("accounts/{id}");
        candidate.accounts.push(StoredAccount {
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
    candidate.active = Some(id.clone());
    let account = to_account(
        candidate
            .accounts
            .iter()
            .find(|account| account.id == id)
            .ok_or_else(|| anyhow!("account was not stored"))?,
    )?;
    write_registry(&candidate)?;
    *guard = candidate;
    Ok(account)
}

/// Select an already-stored account, e.g. F2 offline login.
pub fn activate_stored(server_url: &str, username: &str) -> Result<Account> {
    let id = account_id(server_url, username);
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    let mut candidate = guard.clone();
    let now = crate::consts::now_secs();
    let stored = candidate
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(|| anyhow!("no saved account for {username} @ {server_url}"))?;
    stored.last_used = now;
    let stored = stored.clone();
    candidate.active = Some(id);
    let account = to_account(&stored)?;
    write_registry(&candidate)?;
    *guard = candidate;
    Ok(account)
}

fn is_removable_account_data_root(data_root: &Path, config_dir: &Path) -> bool {
    let accounts_dir = config_dir.join("accounts");
    data_root != config_dir
        && data_root != accounts_dir
        && data_root.starts_with(&accounts_dir)
        && data_root
            .components()
            .all(|comp| !matches!(comp, std::path::Component::ParentDir))
}

/// What an account removal must clean up after the registry was updated.
struct RemovedAccount {
    next: Option<Account>,
    data_root: PathBuf,
    legacy_root: PathBuf,
}

/// Remove the active entry from the registry (sync part of
/// [`remove_active`]): the registry lock is never held across an await point.
fn unregister_active() -> Result<Option<RemovedAccount>> {
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    let Some(active) = guard.active.clone() else {
        return Ok(None);
    };
    let mut candidate = guard.clone();
    let Some(index) = candidate
        .accounts
        .iter()
        .position(|account| account.id == active)
    else {
        candidate.active = None;
        write_registry(&candidate)?;
        *guard = candidate;
        return Ok(None);
    };
    let removed = candidate.accounts.remove(index);
    let next = candidate
        .accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .cloned();
    candidate.active = next.as_ref().map(|account| account.id.clone());
    let next_account = next.as_ref().map(to_account).transpose()?;
    let legacy_root = crate::db::config_dir()?;
    let data_root = resolve_data_root(&removed.data_root)?;
    write_registry(&candidate)?;
    *guard = candidate;
    Ok(Some(RemovedAccount {
        next: next_account,
        data_root,
        legacy_root,
    }))
}

/// Remove the active account. `keep_data` leaves its data root on disk.
/// Returns the next account to activate (or None when no accounts remain).
///
/// This is async because the data root may only be deleted after every SQLite
/// pool of the outgoing account is closed: unlinking files under live handles
/// races the pool teardown and can leave a half-removed directory (S-09).
pub async fn remove_active(keep_data: bool) -> Result<Option<Account>> {
    let Some(removed) = unregister_active()? else {
        return Ok(None);
    };
    crate::app::close_dbs().await;
    if !keep_data && is_removable_account_data_root(&removed.data_root, &removed.legacy_root) {
        _ = std::fs::remove_dir_all(&removed.data_root);
    }
    Ok(removed.next)
}

/// First-run migration: import the legacy single-account credentials table.
pub async fn migrate_legacy(dbs: &Dbs) -> Result<()> {
    if registry_existed() {
        return Ok(());
    }
    let credentials = crate::sqlite::all_credentials(&dbs.credentials).await?;
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    let mut candidate = Registry::versioned();
    let latest_id = credentials
        .iter()
        .max_by_key(|credential| credential.created_at)
        .map(|credential| account_id(&credential.server_url, &credential.username));
    for credential in credentials {
        let id = account_id(&credential.server_url, &credential.username);
        let legacy = latest_id.as_deref() == Some(id.as_str());
        candidate.accounts.push(StoredAccount {
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
            candidate.active = Some(id);
        }
    }
    write_registry(&candidate)?;
    *guard = candidate;
    Ok(())
}

pub fn ensure_data_root(account: &Account) -> Result<()> {
    crate::db::ensure_dir(&account.data_root)
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

    #[test]
    fn is_removable_account_data_root_confines_to_accounts_subdir() {
        let cfg = Path::new("/tmp/musika-cfg");
        assert!(is_removable_account_data_root(
            &cfg.join("accounts").join("acc1"),
            cfg
        ));
        assert!(!is_removable_account_data_root(cfg, cfg));
        assert!(!is_removable_account_data_root(&cfg.join("accounts"), cfg));
        assert!(!is_removable_account_data_root(
            Path::new("/tmp/other"),
            cfg
        ));
        assert!(!is_removable_account_data_root(
            &cfg.join("accounts").join("..").join("other"),
            cfg
        ));
    }

    #[test]
    fn corrupt_registry_marks_existed_and_is_not_overwritten_by_migrate_legacy() {
        if std::env::var_os("MUSIKA_TEST_CORRUPT_REGISTRY_CHILD").is_some() {
            assert!(
                registry_existed(),
                "corrupt accounts.json on disk must still record REGISTRY_EXISTED = true"
            );
            let path = registry_path().unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "{corrupt-json-do-not-overwrite"
            );
            return;
        }

        let dir = std::env::temp_dir().join(format!("musika-corrupt-reg-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reg_file = dir.join("accounts.json");
        std::fs::write(&reg_file, b"{corrupt-json-do-not-overwrite").unwrap();

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .env("MUSIKA_ACCOUNTS_FILE", &reg_file)
            .env("MUSIKA_TEST_CORRUPT_REGISTRY_CHILD", "1")
            .args([
                "--exact",
                "accounts::tests::corrupt_registry_marks_existed_and_is_not_overwritten_by_migrate_legacy",
            ])
            .output()
            .unwrap();
        _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "corrupt registry child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn registry_writes_0600_and_rolls_back_in_memory_on_disk_failure() {
        if std::env::var_os("MUSIKA_TEST_REGISTRY_WRITE_CHILD").is_some() {
            crate::crypto::use_test_key();
            let reg_file = registry_path().unwrap();
            let acc =
                upsert_and_activate("https://music.example", "alice", "pw", "navidrome").unwrap();
            assert_eq!(acc.username, "alice");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&reg_file).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600);
            }

            // Replace the registry path's temp target `.json.tmp` with a directory so
            // the next `write_registry` fails; in-memory state must remain `alice`.
            let tmp_dir = reg_file.with_extension("json.tmp");
            std::fs::create_dir_all(&tmp_dir).unwrap();
            assert!(
                upsert_and_activate("https://music.example", "bob", "pw2", "navidrome").is_err()
            );
            assert_eq!(active_account().unwrap().username, "alice");
            assert!(account_for("https://music.example", "bob").is_none());
            return;
        }

        let dir = std::env::temp_dir().join(format!("musika-reg-write-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reg_file = dir.join("accounts.json");

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .env("MUSIKA_ACCOUNTS_FILE", &reg_file)
            .env("XDG_CONFIG_HOME", &dir)
            .env("MUSIKA_TEST_REGISTRY_WRITE_CHILD", "1")
            .args([
                "--exact",
                "accounts::tests::registry_writes_0600_and_rolls_back_in_memory_on_disk_failure",
            ])
            .output()
            .unwrap();
        _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "registry write child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
