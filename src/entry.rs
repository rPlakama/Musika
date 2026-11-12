//! CLI entry for the `musika` binary.

fn load_dotenv() {
    let Ok(text) = std::fs::read_to_string(".env") else {
        return;
    };
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
        if !key.is_empty() && std::env::var_os(key).is_none() {
            // SAFETY: `set_var` is unsafe because the process environment is
            // global and must not be mutated while another thread reads it.
            // TODO(safety): the call site (`init_backend`) already started the
            // audio warm-up thread and startup instrumentation before
            // `load_dotenv`, so the single-threaded invariant is not currently
            // established; moving dotenv loading ahead of `warm_native()` would
            // make it sound.
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

pub fn run() {
    // Keep glibc from creating a new arena per decoder thread.
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `mallopt` is a plain glibc C function with integer
        // parameters; no pointers cross this boundary.
        unsafe extern "C" {
            fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
        }
        const M_ARENA_MAX: std::ffi::c_int = -8;
        // SAFETY: the declaration above matches glibc's `int mallopt(int,
        // int)`; the call has no memory-safety preconditions and failure is
        // reported through the return value, which is deliberately ignored.
        unsafe {
            _ = mallopt(M_ARENA_MAX, 1);
        }
    }

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
    if args.iter().any(|arg| arg == "--daemon") {
        crate::daemon::run();
    }
    if args.iter().any(|arg| arg == "--ui") {
        crate::ui::run();
        return;
    }

    launch_or_show();
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
    if !wait_for_path(&crate::ipc::daemon_socket_path(), std::time::Duration::from_secs(3)) {
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

fn wait_for_path(path: &std::path::Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    false
}

fn wait_for_socket(path: &std::path::Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    false
}

fn daemon_log_path() -> Option<std::path::PathBuf> {
    crate::db::config_dir().ok().map(|dir| dir.join("musika-daemon.log"))
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
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            if let Ok(file2) = file.try_clone() {
                command.stdout(std::process::Stdio::from(file));
                command.stderr(std::process::Stdio::from(file2));
            }
        }
    }
    let child_setup = || {
        // SAFETY: `setsid(2)` takes no pointers, allocates nothing and returns
        // -1 on failure; it is async-signal-safe, which is what the
        // fork/exec window requires.
        if unsafe { libc::setsid() } == -1 {
            return Err(std::io::Error::last_os_error());
        }
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

/// Initializes the shared backend: startup instrumentation, native audio
/// warm-up, dotenv, the account data root, and the database set.
pub(crate) fn init_backend() -> bool {
    crate::startup::start();
    crate::startup::trace("main");
    crate::player::warm_native();
    load_dotenv();

    let data_root = match crate::accounts::startup_data_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!("accounts: failed to select data root: {err:#}");
            return false;
        }
    };
    crate::db::set_data_root(data_root);
    crate::db::warm_startup();

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
    crate::app::set_dbs(dbs);
    true
}
