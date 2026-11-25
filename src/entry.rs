//! CLI entry for the `musika` binary.

fn is_allowed_dotenv_key(key: &str) -> bool {
    key.starts_with("MUSIKA_")
        && !matches!(key, "MUSIKA_UI_BIN" | "MUSIKA_MPV_BIN" | "MUSIKA_KEY_FILE")
}

fn parse_allowed_dotenv(text: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches(['"', '\'']);
        if !key.is_empty() && is_allowed_dotenv_key(key) {
            entries.push((key.to_string(), value.to_string()));
        }
    }
    entries
}

fn is_owner_only_file(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: `geteuid(2)` takes no pointers, has no preconditions, and
        // always succeeds returning the effective user ID.
        let euid = unsafe { libc::geteuid() };
        meta.uid() == euid && (meta.mode() & 0o022) == 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn dotenv_candidates(
    config_dir: Option<&std::path::Path>,
    allow_cwd: bool,
) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if allow_cwd {
        paths.push(std::path::PathBuf::from(".env"));
    }
    if let Some(dir) = config_dir {
        paths.push(dir.join("env"));
    }
    paths
}

fn read_safe_dotenv_entries(path: &std::path::Path) -> Vec<(String, String)> {
    if !is_owner_only_file(path) {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_allowed_dotenv(&text)
}

fn load_dotenv() {
    let config_dir = crate::db::config_dir().ok();
    let candidates = dotenv_candidates(config_dir.as_deref(), cfg!(debug_assertions));
    for path in candidates {
        for (key, value) in read_safe_dotenv_entries(&path) {
            if std::env::var_os(&key).is_none() {
                // SAFETY: `load_dotenv` is called at the start of `init_backend`
                // strictly before `player::warm_native()` or the tokio runtime
                // spawns any background threads, so the process is still
                // single-threaded when `set_var` mutates the environment.
                unsafe {
                    std::env::set_var(&key, &value);
                }
            }
        }
    }
}

pub fn run() {
    crate::linux::tune_allocator();

    // CLI probe before any database work, so `musika --version` also works
    // on a headless host.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("musika {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    crate::db::warn_legacy_xdg_config_once();
    if args.iter().any(|arg| arg == "--daemon") {
        crate::daemon::run();
    }
    if args.iter().any(|arg| arg == "--ui") {
        exec_ui();
    }

    launch_or_show();
}

/// `musika --ui`: become the sibling GPUI client instead of linking the front
/// end into the core binary.
///
/// The window is a separate process anyway (`musika-ui`), and the core never
/// touches GPUI. `exec` keeps the CLI contract (`musika --ui` still starts the
/// window with the same stdio/env) while letting the linker drop the whole UI
/// stack from the daemon executable.
#[cfg(unix)]
fn exec_ui() -> ! {
    use std::os::unix::process::CommandExt;

    let Some(ui) = crate::daemon::ui_binary_path() else {
        eprintln!("musika: cannot locate the UI binary");
        std::process::exit(1);
    };
    // The UI ignores command-line arguments; forwarding them keeps any future
    // front-end flags working without another launcher special case.
    let error = std::process::Command::new(&ui)
        .args(std::env::args_os().skip(1))
        .exec();
    eprintln!("musika: could not start {}: {error}", ui.display());
    std::process::exit(1);
}

/// Non-Unix fallback: the project targets Linux; keep a buildable stub.
#[cfg(all(not(unix), feature = "gpuui"))]
fn exec_ui() -> ! {
    crate::ui::run();
    std::process::exit(0);
}

/// Non-Unix fallback without GPUI: exit cleanly when built headless.
#[cfg(all(not(unix), not(feature = "gpuui")))]
fn exec_ui() -> ! {
    eprintln!("musika: built without UI support (gpuui feature disabled)");
    std::process::exit(1);
}

/// Default `musika` invocation: connect to the existing core (or start it),
/// then ask it to spawn/raise the GPUI child. The CLI process exits; the core
/// and the child live on.
fn launch_or_show() {
    let ipc = crate::ipc::socket_path();
    if std::os::unix::net::UnixStream::connect(&ipc).is_err() {
        spawn_detached_daemon();
        if !wait_for_socket(&ipc, std::time::Duration::from_secs(8)) {
            eprintln!(
                "musika: the core daemon did not start; see {}",
                daemon_log_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "the daemon log".to_string())
            );
            std::process::exit(1);
        }
    }
    // Probe by connecting: an existing but stale socket file must not count as
    // ready (D-26).
    if !wait_for_socket(
        &crate::ipc::daemon_socket_path(),
        std::time::Duration::from_secs(3),
    ) {
        eprintln!("musika: the core UI socket is not ready");
        std::process::exit(1);
    }
    for _ in 0..20 {
        if crate::ipc::send_line("show") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("musika: could not ask the core to show the window");
    std::process::exit(1);
}

fn wait_for_socket(path: &std::path::Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn daemon_log_path() -> Option<std::path::PathBuf> {
    crate::db::config_dir()
        .ok()
        .map(|dir| dir.join("musika-daemon.log"))
}

fn spawn_detached_daemon() {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("musika: cannot locate the executable");
        return;
    };
    let mut command = std::process::Command::new(exe);
    command.arg("--daemon");
    command.stdin(std::process::Stdio::null());
    if let Some(log) = daemon_log_path() {
        if let Some(parent) = log.parent() {
            _ = std::fs::create_dir_all(parent);
        }
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            && let Ok(file2) = file.try_clone()
        {
            command.stdout(std::process::Stdio::from(file));
            command.stderr(std::process::Stdio::from(file2));
        }
    }
    let child_setup = || {
        // SAFETY: `setsid(2)` takes no pointers, allocates nothing and returns
        // -1 on failure; it is async-signal-safe, which is what the
        // fork/exec window requires.
        if unsafe { libc::setsid() } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        // The launcher's descriptors (and anything inherited from a terminal
        // or shell) must not survive into the long-lived core.
        crate::linux::close_inherited_fds()?;
        Ok(())
    };
    // SAFETY: `pre_exec` requires the closure to be async-signal-safe in the
    // forked child; `child_setup` only calls `setsid(2)` and
    // `last_os_error()`, holds no locks and allocates nothing.
    unsafe {
        command.pre_exec(child_setup);
    }
    match command.spawn() {
        Ok(child) => {
            eprintln!("musika: core daemon started (pid {})", child.id());
        }
        Err(error) => {
            eprintln!("musika: could not start the core daemon: {error}");
            std::process::exit(1);
        }
    }
}

/// Initializes the shared backend: startup instrumentation, dotenv, legacy
/// XDG config check, native audio warm-up, the account data root, and the
/// database set.
pub(crate) fn init_backend() -> bool {
    crate::startup::start();
    load_dotenv();
    crate::startup::trace("main");
    crate::db::warn_legacy_xdg_config_once();
    crate::player::warm_native();

    let data_root = match crate::accounts::startup_data_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!("accounts: failed to select data root: {err:#}");
            return false;
        }
    };
    crate::startup::trace("accounts.root");
    crate::db::set_data_root(data_root);

    let dbs = match crate::app::runtime().block_on(crate::db::init()) {
        Ok(dbs) => dbs,
        Err(err) => {
            eprintln!("database: failed to initialise: {err:#}");
            return false;
        }
    };
    crate::startup::trace("db.init");
    if let Err(err) = crate::app::runtime().block_on(crate::accounts::migrate_legacy(&dbs)) {
        eprintln!("accounts: legacy migration failed: {err:#}");
    }
    crate::startup::trace("accounts.migrate");
    crate::app::set_dbs(dbs);
    crate::startup::trace("db.ready");
    // Warm after the migrations so the page cache holds the final files; the
    // thread is demoted and only issues kernel readahead.
    crate::db::warm_startup();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_allows_only_safe_musika_keys() {
        let text = "\
            # comment\n\
            MUSIKA_UI_BIN=/tmp/evil\n\
            MUSIKA_MPV_BIN=/tmp/evil-mpv\n\
            MUSIKA_KEY_FILE=/tmp/evil-key\n\
            LD_PRELOAD=/tmp/evil.so\n\
            LD_LIBRARY_PATH=/tmp/evil-lib\n\
            PATH=/tmp/evil-bin\n\
            XDG_CONFIG_HOME=/tmp/evil-cfg\n\
            DATABASE_URL=\"sqlite:app.db\"\n\
            export MUSIKA_STARTUP_TRACE=\"1\"\n\
            MUSIKA_OUTPUT_BACKEND='alsa'\n\
            MUSIKA_SEARCH_ENGINE=fts\n";

        let entries = parse_allowed_dotenv(text);
        assert_eq!(
            entries,
            vec![
                ("MUSIKA_STARTUP_TRACE".to_string(), "1".to_string()),
                ("MUSIKA_OUTPUT_BACKEND".to_string(), "alsa".to_string()),
                ("MUSIKA_SEARCH_ENGINE".to_string(), "fts".to_string()),
            ]
        );
    }

    #[test]
    fn dotenv_candidates_excludes_cwd_when_disallowed() {
        let cfg = std::path::Path::new("/tmp/musika-cfg");
        assert_eq!(dotenv_candidates(Some(cfg), false), vec![cfg.join("env")]);
        assert_eq!(
            dotenv_candidates(Some(cfg), true),
            vec![std::path::PathBuf::from(".env"), cfg.join("env")]
        );
        assert!(dotenv_candidates(None, false).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn dotenv_rejects_symlinks_and_group_or_world_writable_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = std::env::temp_dir().join(format!(
            "musika-entry-dotenv-{}-{}",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let safe_file = dir.join("env");
        std::fs::write(
            &safe_file,
            "MUSIKA_UI_BIN=/evil\nMUSIKA_STARTUP_TRACE=1\nLD_PRELOAD=/evil.so\n",
        )
        .expect("write safe env");
        std::fs::set_permissions(&safe_file, std::fs::Permissions::from_mode(0o600))
            .expect("chmod 600");

        assert_eq!(
            read_safe_dotenv_entries(&safe_file),
            vec![("MUSIKA_STARTUP_TRACE".to_string(), "1".to_string())]
        );

        let link = dir.join("env.link");
        symlink(&safe_file, &link).expect("symlink");
        assert!(
            read_safe_dotenv_entries(&link).is_empty(),
            "symlinks must be rejected"
        );

        std::fs::set_permissions(&safe_file, std::fs::Permissions::from_mode(0o666))
            .expect("chmod 666");
        assert!(
            read_safe_dotenv_entries(&safe_file).is_empty(),
            "group/world-writable files must be rejected"
        );

        _ = std::fs::remove_dir_all(&dir);
    }
}
